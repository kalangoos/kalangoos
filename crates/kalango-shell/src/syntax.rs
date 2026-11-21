//! Syntax colours for the code in an answer (design "Chat · diagramas, documentos e realce de sintaxe"): a small
//! lexer of our own, one line at a time with the state a line leaves for the next (a block comment, a string that
//! goes on, an open tag), so a block that grows only colours its new lines. No grammar files and nothing to load:
//! keywords, strings, numbers, called functions, types, comments, tags, attributes and punctuation, for the languages
//! models write in chats. A language it does not know stays plain.

/// What a piece of a line is (its colour: keyword accent, text teal, number and attribute gold, function blue, type
/// and tag coral, comment dim, punctuation muted, plain text2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Plain,
    Keyword,
    Text,
    Number,
    Function,
    Type,
    Comment,
    Tag,
    Attribute,
    Punct,
}

/// The languages, as families that lex alike.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Rust,
    Python,
    /// JavaScript and TypeScript; `jsx` also reads tags
    Js { jsx: bool },
    C,
    Go,
    Java,
    Shell,
    Sql,
    Html,
    Css,
    Json,
    Yaml,
    Toml,
    Markdown,
    Latex,
    Mermaid,
}

/// The language a fence names (None: plain text).
pub fn lang(name: &str) -> Option<Lang> {
    Some(match name.to_lowercase().as_str() {
        "rs" | "rust" => Lang::Rust,
        "py" | "python" | "python3" => Lang::Python,
        "js" | "javascript" | "mjs" | "cjs" | "ts" | "typescript" | "node" => Lang::Js { jsx: false },
        "jsx" | "tsx" | "react" => Lang::Js { jsx: true },
        "c" | "h" | "cpp" | "c++" | "cc" | "hpp" | "cxx" => Lang::C,
        "go" | "golang" => Lang::Go,
        "java" | "kotlin" | "kt" | "cs" | "csharp" | "c#" => Lang::Java,
        "sh" | "bash" | "shell" | "zsh" | "console" | "terminal" | "fish" => Lang::Shell,
        "sql" | "sqlite" | "postgres" | "postgresql" | "mysql" => Lang::Sql,
        "html" | "htm" | "xml" | "svg" | "xhtml" | "vue" => Lang::Html,
        "css" | "scss" | "less" => Lang::Css,
        "json" | "jsonc" | "json5" => Lang::Json,
        "yaml" | "yml" => Lang::Yaml,
        "toml" | "ini" | "cfg" | "conf" => Lang::Toml,
        "md" | "markdown" => Lang::Markdown,
        "latex" | "tex" => Lang::Latex,
        "mermaid" | "mmd" => Lang::Mermaid,
        _ => return None,
    })
}

const RUST: &[&str] = &["as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while"];
const PYTHON: &[&str] = &["and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "self", "True", "try", "while", "with", "yield", "match", "case"];
const JS: &[&str] = &["as", "async", "await", "break", "case", "catch", "class", "const", "continue", "default", "delete", "do", "else", "export", "extends", "false", "finally", "for", "from", "function", "if", "import", "in", "instanceof", "let", "new", "null", "of", "return", "static", "super", "switch", "this", "throw", "true", "try", "typeof", "undefined", "var", "void", "while", "yield", "interface", "type", "enum", "implements", "readonly", "public", "private", "protected", "keyof", "declare", "namespace"];
const C: &[&str] = &["auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else", "enum", "extern", "float", "for", "goto", "if", "int", "long", "register", "return", "short", "signed", "sizeof", "static", "struct", "switch", "typedef", "union", "unsigned", "void", "volatile", "while", "class", "public", "private", "protected", "namespace", "template", "typename", "true", "false", "nullptr", "new", "delete", "bool", "auto", "using", "virtual", "override", "include", "define", "ifdef", "ifndef", "endif", "pragma"];
const GO: &[&str] = &["break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for", "func", "go", "goto", "if", "import", "interface", "map", "package", "range", "return", "select", "struct", "switch", "type", "var", "true", "false", "nil"];
const JAVA: &[&str] = &["abstract", "boolean", "break", "byte", "case", "catch", "char", "class", "const", "continue", "default", "do", "double", "else", "enum", "extends", "final", "finally", "float", "for", "if", "implements", "import", "instanceof", "int", "interface", "long", "new", "package", "private", "protected", "public", "return", "short", "static", "super", "switch", "this", "throw", "throws", "try", "void", "while", "true", "false", "null", "var", "val", "fun", "override", "using", "namespace", "string", "record"];
const SHELL: &[&str] = &["if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "in", "function", "return", "local", "export", "sudo", "cd", "echo", "exit", "source", "set", "unset", "readonly", "shift", "true", "false"];
const SQL: &[&str] = &["select", "from", "where", "and", "or", "not", "insert", "into", "values", "update", "set", "delete", "create", "table", "drop", "alter", "add", "index", "on", "join", "left", "right", "inner", "outer", "group", "by", "order", "having", "limit", "offset", "as", "distinct", "primary", "key", "foreign", "references", "null", "is", "in", "like", "between", "union", "all", "exists", "case", "when", "then", "else", "end", "default", "unique", "integer", "int", "text", "varchar", "boolean", "date", "timestamp", "real", "serial", "asc", "desc", "count", "sum", "avg", "min", "max", "true", "false", "begin", "commit", "view", "with"];
const LITERALS: &[&str] = &["true", "false", "null", "yes", "no", "on", "off", "~"];
const MERMAID: &[&str] = &["flowchart", "graph", "sequenceDiagram", "classDiagram", "stateDiagram", "stateDiagram-v2", "erDiagram", "gantt", "pie", "journey", "mindmap", "timeline", "gitGraph", "TD", "TB", "LR", "RL", "BT", "subgraph", "end", "participant", "actor", "as", "Note", "note", "over", "left", "right", "of", "loop", "alt", "else", "opt", "par", "and", "critical", "break", "rect", "autonumber", "activate", "deactivate", "title", "section", "classDef", "class", "style", "linkStyle", "click", "direction", "showData", "dateFormat"];

fn keywords(l: Lang) -> &'static [&'static str] {
    match l {
        Lang::Rust => RUST,
        Lang::Python => PYTHON,
        Lang::Js { .. } => JS,
        Lang::C => C,
        Lang::Go => GO,
        Lang::Java => JAVA,
        Lang::Shell => SHELL,
        Lang::Sql => SQL,
        Lang::Mermaid => MERMAID,
        Lang::Json | Lang::Yaml | Lang::Toml => LITERALS,
        _ => &[],
    }
}

fn line_comments(l: Lang) -> &'static [&'static str] {
    match l {
        Lang::Python | Lang::Shell | Lang::Yaml | Lang::Toml => &["#"],
        Lang::Sql => &["--"],
        Lang::Latex => &["%"],
        Lang::Mermaid => &["%%"],
        Lang::Rust | Lang::Js { .. } | Lang::C | Lang::Go | Lang::Java | Lang::Json => &["//"],
        _ => &[],
    }
}

fn block_comment(l: Lang) -> Option<(&'static str, &'static str)> {
    match l {
        Lang::Rust | Lang::Js { .. } | Lang::C | Lang::Go | Lang::Java | Lang::Css | Lang::Json | Lang::Sql => Some(("/*", "*/")),
        Lang::Html | Lang::Markdown => Some(("<!--", "-->")),
        _ => None,
    }
}

/// What a line leaves open for the next.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum State {
    #[default]
    Normal,
    /// inside a block comment, until this
    Comment(&'static str),
    /// inside a string that spans lines (""" or ''' or `), until this
    Str(&'static str),
    /// inside a tag, between its attributes
    InTag,
}

/// Colours code one line at a time; `line` keeps the state for the next one.
pub struct Highlighter {
    lang: Lang,
    pub state: State,
}

impl Highlighter {
    pub fn new(lang: Lang) -> Self {
        Highlighter { lang, state: State::Normal }
    }

    /// The pieces of the next line, with their kinds (same kinds next to each other joined).
    pub fn line(&mut self, line: &str) -> Vec<(Kind, String)> {
        let mut out = Vec::new();
        let ch: Vec<char> = line.chars().collect();
        let mut i = 0;
        let l = self.lang;
        let kw = keywords(l);
        let s = |a: usize, b: usize| ch[a..b].iter().collect::<String>();
        let starts = |i: usize, p: &str| p.chars().enumerate().all(|(k, c)| ch.get(i + k) == Some(&c));
        // the first word of a line, for languages where it means something (YAML/TOML keys, a Markdown heading)
        let lead = ch.iter().take_while(|c| c.is_whitespace()).count();
        if l == Lang::Markdown && self.state == State::Normal {
            let t: String = ch[lead..].iter().collect();
            if t.starts_with('#') {
                push(&mut out, Kind::Keyword, line.to_string());
                return out;
            }
            if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("> ") || t.split_once(". ").is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())) {
                let k = t.find(' ').unwrap_or(0) + 1;
                push(&mut out, Kind::Plain, s(0, lead));
                push(&mut out, Kind::Punct, t[..k].to_string());
                i = lead + t[..k].chars().count();
            }
        }
        while i < ch.len() {
            // carried over from the line before
            match self.state {
                State::Comment(end) => {
                    let mut j = i;
                    while j < ch.len() && !starts(j, end) {
                        j += 1;
                    }
                    let stop = (j + end.chars().count()).min(ch.len());
                    push(&mut out, Kind::Comment, s(i, stop));
                    if j < ch.len() {
                        self.state = State::Normal;
                    }
                    i = stop;
                    continue;
                }
                State::Str(end) => {
                    let mut j = i;
                    while j < ch.len() && !starts(j, end) {
                        j += if ch[j] == '\\' { 2 } else { 1 };
                    }
                    let stop = (j + end.chars().count()).min(ch.len());
                    push(&mut out, Kind::Text, s(i, stop));
                    if j < ch.len() {
                        self.state = State::Normal;
                    }
                    i = stop;
                    continue;
                }
                State::InTag => {
                    let c = ch[i];
                    if c == '>' || starts(i, "/>") {
                        let n = if c == '>' { 1 } else { 2 };
                        push(&mut out, Kind::Punct, s(i, i + n));
                        i += n;
                        self.state = State::Normal;
                    } else if c == '"' || c == '\'' {
                        let j = string_end(&ch, i, c);
                        push(&mut out, Kind::Text, s(i, j));
                        i = j;
                    } else if c == '{' && matches!(l, Lang::Js { .. }) {
                        // a JSX expression in an attribute: its braces, the rest as code
                        let mut depth = 0;
                        let mut j = i;
                        while j < ch.len() {
                            if ch[j] == '{' {
                                depth += 1;
                            } else if ch[j] == '}' {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            j += 1;
                        }
                        push(&mut out, Kind::Punct, "{".into());
                        let inner: String = s(i + 1, j.min(ch.len()));
                        let mut sub = Highlighter::new(Lang::Js { jsx: false });
                        for (k, t) in sub.line(&inner) {
                            push(&mut out, k, t);
                        }
                        if j < ch.len() {
                            push(&mut out, Kind::Punct, "}".into());
                        }
                        i = (j + 1).min(ch.len());
                    } else if c.is_alphabetic() || c == '_' || c == ':' || c == '@' {
                        let mut j = i;
                        while j < ch.len() && (ch[j].is_alphanumeric() || matches!(ch[j], '-' | '_' | ':' | '.' | '@')) {
                            j += 1;
                        }
                        push(&mut out, Kind::Attribute, s(i, j));
                        i = j;
                    } else {
                        push(&mut out, if c == '=' { Kind::Punct } else { Kind::Plain }, c.to_string());
                        i += 1;
                    }
                    continue;
                }
                State::Normal => {}
            }
            let c = ch[i];
            // comments
            if let Some((open, close)) = block_comment(l) {
                if starts(i, open) {
                    self.state = State::Comment(close);
                    push(&mut out, Kind::Comment, open.to_string());
                    i += open.chars().count();
                    continue;
                }
            }
            if line_comments(l).iter().any(|p| starts(i, p)) && !(l == Lang::Latex && i > 0 && ch[i - 1] == '\\') && !(l == Lang::Shell && i > 0 && !ch[i - 1].is_whitespace()) {
                push(&mut out, Kind::Comment, s(i, ch.len()));
                break;
            }
            // tags (HTML, XML, JSX, a Markdown page's HTML)
            let tags = matches!(l, Lang::Html | Lang::Markdown) || l == (Lang::Js { jsx: true });
            if tags && c == '<' && ch.get(i + 1).is_some_and(|n| n.is_alphabetic() || *n == '/' || *n == '!' || *n == '>') {
                let n = if ch.get(i + 1) == Some(&'/') { 2 } else { 1 };
                push(&mut out, Kind::Punct, s(i, i + n));
                let mut j = i + n;
                while j < ch.len() && (ch[j].is_alphanumeric() || matches!(ch[j], '-' | '_' | ':' | '.' | '!')) {
                    j += 1;
                }
                push(&mut out, Kind::Tag, s(i + n, j));
                i = j;
                self.state = State::InTag;
                continue;
            }
            // strings
            if matches!(l, Lang::Python) && (starts(i, "\"\"\"") || starts(i, "'''")) {
                let q: &'static str = if c == '"' { "\"\"\"" } else { "'''" };
                push(&mut out, Kind::Text, q.to_string());
                i += 3;
                self.state = State::Str(q);
                continue;
            }
            if c == '`' && matches!(l, Lang::Js { .. } | Lang::Go) {
                push(&mut out, Kind::Text, "`".into());
                i += 1;
                self.state = State::Str("`");
                continue;
            }
            let quote = c == '"' || (c == '\'' && !matches!(l, Lang::Rust | Lang::Markdown | Lang::Latex | Lang::Mermaid)) || (c == '`' && matches!(l, Lang::Markdown | Lang::Shell));
            // (a Rust char literal 'x' or '\n', not a lifetime)
            let rust_char = l == Lang::Rust && c == '\'' && (ch.get(i + 2) == Some(&'\'') || (ch.get(i + 1) == Some(&'\\') && ch[i + 2..].contains(&'\'')));
            if quote || rust_char {
                let j = string_end(&ch, i, c);
                // a JSON/YAML key ("name": …) is an attribute
                let key = matches!(l, Lang::Json | Lang::Yaml) && ch[j..].iter().find(|c| !c.is_whitespace()) == Some(&':');
                push(&mut out, if key { Kind::Attribute } else { Kind::Text }, s(i, j));
                i = j;
                continue;
            }
            // LaTeX commands, shell variables, decorators and attributes
            if l == Lang::Latex && c == '\\' {
                let mut j = i + 1;
                while j < ch.len() && ch[j].is_alphabetic() {
                    j += 1;
                }
                if j == i + 1 {
                    j = (i + 2).min(ch.len());
                }
                if ch.get(j) == Some(&'*') {
                    j += 1;
                }
                push(&mut out, Kind::Keyword, s(i, j));
                i = j;
                continue;
            }
            if l == Lang::Latex && c == '$' {
                let j = ch[i + 1..].iter().position(|x| *x == '$').map(|k| i + k + 2).unwrap_or(ch.len());
                push(&mut out, Kind::Text, s(i, j));
                i = j;
                continue;
            }
            if l == Lang::Shell && c == '$' {
                let mut j = i + 1;
                if ch.get(j) == Some(&'{') {
                    while j < ch.len() && ch[j] != '}' {
                        j += 1;
                    }
                    j = (j + 1).min(ch.len());
                } else {
                    while j < ch.len() && (ch[j].is_alphanumeric() || ch[j] == '_' || (j == i + 1 && matches!(ch[j], '@' | '?' | '#' | '!' | '$'))) {
                        j += 1;
                    }
                }
                push(&mut out, Kind::Attribute, s(i, j));
                i = j;
                continue;
            }
            if l == Lang::Shell && c == '-' && (i == 0 || ch[i - 1].is_whitespace()) && ch.get(i + 1).is_some_and(|n| n.is_alphabetic() || *n == '-') {
                let mut j = i;
                while j < ch.len() && !ch[j].is_whitespace() && ch[j] != '=' {
                    j += 1;
                }
                push(&mut out, Kind::Attribute, s(i, j));
                i = j;
                continue;
            }
            if (c == '@' && matches!(l, Lang::Python | Lang::Java | Lang::Js { .. } | Lang::Css)) || (c == '#' && matches!(l, Lang::Rust | Lang::C) && ch.get(i + 1).is_some_and(|n| *n == '[' || n.is_alphabetic())) {
                let mut j = i + 1;
                while j < ch.len() && (ch[j].is_alphanumeric() || matches!(ch[j], '_' | '.' | '-' | '[' | '!')) {
                    j += 1;
                }
                push(&mut out, if l == Lang::Css { Kind::Keyword } else { Kind::Type }, s(i, j));
                i = j;
                continue;
            }
            // CSS: selectors, properties, colours and units
            if l == Lang::Css {
                if c == '#' || (c == '.' && ch.get(i + 1).is_some_and(|n| n.is_alphabetic())) {
                    let mut j = i + 1;
                    while j < ch.len() && (ch[j].is_alphanumeric() || matches!(ch[j], '-' | '_')) {
                        j += 1;
                    }
                    let colour = c == '#' && (j - i == 4 || j - i == 7 || j - i == 9) && ch[i + 1..j].iter().all(char::is_ascii_hexdigit);
                    push(&mut out, if colour { Kind::Number } else { Kind::Tag }, s(i, j));
                    i = j;
                    continue;
                }
                if c.is_alphabetic() || c == '-' {
                    let mut j = i;
                    while j < ch.len() && (ch[j].is_alphanumeric() || matches!(ch[j], '-' | '_')) {
                        j += 1;
                    }
                    let next = ch[j..].iter().find(|c| !c.is_whitespace()).copied();
                    let property = next == Some(':') && ch[..i].iter().rev().find(|c| !c.is_whitespace()).is_some_and(|p| matches!(p, '{' | ';')) || (next == Some(':') && lead == i);
                    let kind = if property { Kind::Attribute } else if next == Some('(') { Kind::Function } else if next == Some('{') || next == Some(',') { Kind::Tag } else { Kind::Plain };
                    push(&mut out, kind, s(i, j));
                    i = j;
                    continue;
                }
            }
            if c.is_ascii_digit() || (c == '.' && ch.get(i + 1).is_some_and(char::is_ascii_digit) && !ch.get(i.wrapping_sub(1)).is_some_and(|p| p.is_alphanumeric())) {
                let mut j = i;
                while j < ch.len() && (ch[j].is_ascii_alphanumeric() || ch[j] == '.' || ch[j] == '_' || ch[j] == '%') {
                    j += 1;
                }
                push(&mut out, Kind::Number, s(i, j));
                i = j;
                continue;
            }
            if c.is_alphabetic() || c == '_' {
                let mut j = i;
                while j < ch.len() && (ch[j].is_alphanumeric() || ch[j] == '_' || (matches!(l, Lang::Yaml | Lang::Toml | Lang::Mermaid) && ch[j] == '-' && ch.get(j + 1).is_some_and(|n| n.is_alphanumeric()))) {
                    j += 1;
                }
                let word = s(i, j);
                let next = ch[j..].iter().find(|c| !c.is_whitespace()).copied();
                let rust_macro = l == Lang::Rust && ch.get(j) == Some(&'!') && ch.get(j + 1) != Some(&'=');
                let is_kw = if l == Lang::Sql { kw.contains(&word.to_lowercase().as_str()) } else { kw.contains(&word.as_str()) };
                let kind = if matches!(l, Lang::Yaml | Lang::Toml) && i == lead && (next == Some(':') || next == Some('=')) {
                    Kind::Attribute
                } else if l == Lang::Toml && ch.get(lead) == Some(&'[') {
                    Kind::Tag
                } else if is_kw {
                    Kind::Keyword
                } else if rust_macro {
                    Kind::Type
                } else if matches!(l, Lang::Markdown | Lang::Latex | Lang::Mermaid | Lang::Yaml | Lang::Toml | Lang::Json | Lang::Html) {
                    Kind::Plain
                } else if word.chars().next().is_some_and(char::is_uppercase) && l != Lang::Shell && l != Lang::Sql {
                    // (a capital name is a type, a component or a constructor, called or not: coral)
                    Kind::Type
                } else if next == Some('(') {
                    Kind::Function
                } else {
                    Kind::Plain
                };
                push(&mut out, kind, word);
                i = j;
                if rust_macro {
                    push(&mut out, Kind::Type, "!".into());
                    i += 1;
                }
                continue;
            }
            // Mermaid arrows, the rest's punctuation
            if l == Lang::Mermaid && matches!(c, '-' | '=' | '.' | '<' | '>') {
                let mut j = i;
                while j < ch.len() && matches!(ch[j], '-' | '=' | '.' | '<' | '>') {
                    j += 1;
                }
                push(&mut out, Kind::Punct, s(i, j));
                i = j;
                continue;
            }
            let punct = "{}()[];,.:=+-*/<>!&|?%^~".contains(c);
            push(&mut out, if punct { Kind::Punct } else { Kind::Plain }, c.to_string());
            i += 1;
        }
        // (a Python/JS line that ends in an open string that cannot span lines closes it: the state stays Normal)
        out
    }
}

/// Where a string that opens at `i` with `q` ends (after its closing quote, or the line's end).
fn string_end(ch: &[char], i: usize, q: char) -> usize {
    let mut j = i + 1;
    while j < ch.len() && ch[j] != q {
        if ch[j] == '\\' {
            j += 1;
        }
        j += 1;
    }
    (j + 1).min(ch.len())
}

fn push(out: &mut Vec<(Kind, String)>, k: Kind, s: String) {
    if s.is_empty() {
        return;
    }
    let k = if s.chars().all(char::is_whitespace) { Kind::Plain } else { k };
    if let Some(last) = out.last_mut().filter(|l| l.0 == k) {
        last.1.push_str(&s);
    } else {
        out.push((k, s));
    }
}

/// A whole block, line by line (each line's pieces).
pub fn highlight(lang: Option<Lang>, code: &str) -> Vec<Vec<(Kind, String)>> {
    match lang {
        None => code.lines().map(|l| vec![(Kind::Plain, l.to_string())]).collect(),
        Some(lang) => {
            let mut h = Highlighter::new(lang);
            code.lines().map(|l| h.line(l)).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pieces of a kind in a line, as written.
    fn of(lang: &str, code: &str, k: Kind) -> Vec<String> {
        highlight(super::lang(lang), code).into_iter().flatten().filter(|p| p.0 == k).map(|p| p.1.trim().to_string()).filter(|p| !p.is_empty()).collect()
    }

    #[test]
    fn jsx_from_the_design() {
        let src = "// a contagem de cliques\nimport { useState } from \"react\";\nexport function Contador({ inicio = 0 }) {\n  const [n, setN] = useState(inicio);\n  return (\n    <button className=\"cta\" onClick={() => setN(n + 1)}>\n      Cliques: {n}\n    </button>\n  );\n}";
        assert_eq!(of("jsx", src, Kind::Comment), ["// a contagem de cliques"]);
        assert!(of("jsx", src, Kind::Keyword).iter().any(|k| k == "import"));
        assert!(of("jsx", src, Kind::Text).contains(&"\"react\"".to_string()));
        assert!(of("jsx", src, Kind::Text).contains(&"\"cta\"".to_string()));
        assert_eq!(of("jsx", src, Kind::Tag), ["button", "button"]);
        assert_eq!(of("jsx", src, Kind::Attribute), ["className", "onClick"]);
        assert!(of("jsx", src, Kind::Function).contains(&"useState".to_string()));
        assert!(of("jsx", src, Kind::Function).contains(&"setN".to_string()));
        assert!(of("jsx", src, Kind::Type).contains(&"Contador".to_string()));
        assert!(of("jsx", src, Kind::Number).contains(&"1".to_string()));
        // plain JS: a comparison is not a tag
        assert!(of("js", "if (a < b) { x = 1 }", Kind::Tag).is_empty());
    }

    #[test]
    fn html_and_css() {
        let src = "<!-- topo -->\n<a class=\"cta\" href='/baixar'>Baixar</a>\n<img\n  src=\"x.png\"\n  alt=\"logo\" />";
        assert_eq!(of("html", src, Kind::Comment), ["<!-- topo -->"]);
        assert_eq!(of("html", src, Kind::Tag), ["a", "a", "img"]);
        // attributes on the lines after the tag's name (the state carries over)
        assert_eq!(of("html", src, Kind::Attribute), ["class", "href", "src", "alt"]);
        assert_eq!(of("html", src, Kind::Plain), ["Baixar"]);
        let css = ".cta { color: #A5A1E2; margin: 0 4px; }\n/* um\n   comentário */\nbody{font-family:Inter}";
        assert_eq!(of("css", css, Kind::Tag), [".cta", "body"]);
        assert_eq!(of("css", css, Kind::Attribute), ["color", "margin", "font-family"]);
        assert_eq!(of("css", css, Kind::Number), ["#A5A1E2", "0", "4px"]);
        assert_eq!(of("css", css, Kind::Comment), ["/* um", "comentário */"]);
    }

    #[test]
    fn data_files() {
        let json = "{\"nome\": \"Kalango\", \"versao\": 2, \"ativo\": true}";
        assert_eq!(of("json", json, Kind::Attribute), ["\"nome\"", "\"versao\"", "\"ativo\""]);
        assert_eq!(of("json", json, Kind::Text), ["\"Kalango\""]);
        assert_eq!(of("json", json, Kind::Keyword), ["true"]);
        let yaml = "# config\nservidor:\n  porta: 8080\n  nome: \"teste\"";
        assert_eq!(of("yaml", yaml, Kind::Attribute), ["servidor", "porta", "nome"]);
        assert_eq!(of("yaml", yaml, Kind::Number), ["8080"]);
        let toml = "[package]\nname = \"kalango\"\nedition = 2021";
        assert_eq!(of("toml", toml, Kind::Tag), ["package"]);
        assert_eq!(of("toml", toml, Kind::Attribute), ["name", "edition"]);
    }

    #[test]
    fn languages_that_carry_state() {
        // a Python docstring over three lines
        let py = "def f(x):\n    \"\"\"Soma.\n    Mais.\"\"\"\n    return x + 1  # fim";
        assert_eq!(of("python", py, Kind::Text), ["\"\"\"Soma.", "Mais.\"\"\""]);
        assert_eq!(of("python", py, Kind::Comment), ["# fim"]);
        assert!(of("python", py, Kind::Function).contains(&"f".to_string()));
        // a C block comment over two lines, a Rust lifetime is not a string
        assert_eq!(of("c", "/* a\n b */ int x = 2;", Kind::Comment), ["/* a", "b */"]);
        assert!(of("rust", "fn f<'a>(x: &'a str) -> char { 'z' }", Kind::Text) == ["'z'"]);
        assert!(of("rust", "println!(\"oi\");", Kind::Type).contains(&"println!".to_string()));
    }

    #[test]
    fn shell_sql_go_java_markdown_latex_mermaid() {
        let sh = "export PATH=\"$HOME/bin:$PATH\"\ncargo build --release # tudo\necho ${USER}";
        assert_eq!(of("bash", sh, Kind::Attribute), ["--release", "${USER}"]);
        assert_eq!(of("bash", sh, Kind::Comment), ["# tudo"]);
        assert_eq!(of("sql", "SELECT nome FROM pessoas WHERE idade > 18; -- adultos", Kind::Keyword), ["SELECT", "FROM", "WHERE"]);
        assert!(of("go", "func main() { fmt.Println(\"oi\") }", Kind::Keyword).contains(&"func".to_string()));
        assert!(of("java", "public class App { void run() {} }", Kind::Type).contains(&"App".to_string()));
        assert_eq!(of("markdown", "# Título\n- item com `code`", Kind::Keyword), ["# Título"]);
        assert_eq!(of("markdown", "# Título\n- item com `code`", Kind::Text), ["`code`"]);
        let tex = "% forum\n\\begin{document}\n\\item \\textbf{Post} custa 5\\% $x^2$";
        assert_eq!(of("latex", tex, Kind::Comment), ["% forum"]);
        assert_eq!(of("latex", tex, Kind::Keyword), ["\\begin", "\\item", "\\textbf", "\\%"]);
        assert_eq!(of("latex", tex, Kind::Text), ["$x^2$"]);
        let mm = "flowchart TD\n  A[Iniciar] --> B{Passou?}\n  %% nota";
        assert_eq!(of("mermaid", mm, Kind::Keyword), ["flowchart", "TD"]);
        assert!(of("mermaid", mm, Kind::Punct).contains(&"-->".to_string()));
        assert_eq!(of("mermaid", mm, Kind::Comment), ["%% nota"]);
        // an unknown language stays plain
        assert!(highlight(lang("brainfuck"), "+++.").iter().flatten().all(|p| p.0 == Kind::Plain));
    }

    #[test]
    fn a_line_at_a_time_is_the_same_as_the_whole() {
        // (a growing block colours only its new lines: the state between lines is all it needs)
        let src = "/* a\n b */\nconst s = `x\ny`;\n<div class=\"a\"\n  id=\"b\">";
        let whole = highlight(lang("jsx"), src);
        let mut h = Highlighter::new(lang("jsx").unwrap());
        let parts: Vec<_> = src.lines().map(|l| h.line(l)).collect();
        assert_eq!(whole, parts);
    }

    #[test]
    fn five_hundred_lines_fast() {
        let block: String = (0..500).map(|i| format!("    let v{i} = compute(\"texto {i}\", {i}) + x.len(); // linha {i}\n")).collect();
        let t = std::time::Instant::now();
        let lines = highlight(lang("rust"), &block);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(lines.len(), 500);
        eprintln!("realce: 500 linhas em {ms:.2} ms");
        assert!(ms < 200.0, "{ms} ms");
    }
}
