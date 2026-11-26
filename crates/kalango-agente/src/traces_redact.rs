//! Redaction at the source for the trace capture (docs/INGEST.md of Kalango Train, section 5): everything a span
//! carries goes through here before it leaves the process. What it never lets through: secrets (tokens, API keys,
//! passwords, `Authorization` headers, cookies, private keys), environment variables, and any string above 64 KiB
//! (only its size and hash go). The receiver has its own detector; this one is the contract, that one the safety net.
//!
//! No `regex`: the scans are plain loops over the text, so a long tool result costs one pass per detector.

use serde_json::Value;

/// The key names whose values are never sent (compared lower-cased, without `_`, `-` or spaces, as a substring:
/// `X-Api-Key`, `GITHUB_TOKEN` and `dbPassword` all match). Numbers and booleans under such a key stay (`max_tokens`).
pub const DENY_KEYS: &[&str] = &[
    "password",
    "passwd",
    "token",
    "secret",
    "authorization",
    "api_key",
    "apikey",
    "passphrase",
    "private_key",
    "cookie",
    "credential",
    "senha",
    "segredo",
    // the same names in the other languages of the interface
    "contraseña",
    "contrasena",
    "mot de passe",
    "passwort",
    "kennwort",
    "parola d'ordine",
    "пароль",
    "パスワード",
    "密码",
    "密碼",
    "口令",
];

/// Short key names that are secrets only as a whole word at the end of the key (`pwd`, `db_pass`, `X-Auth`, `userPin`),
/// never as a piece of another word (`author`, `passageiro`, `mapping`, `spin`).
pub const SUFFIX_KEYS: &[&str] = &["pwd", "pass", "auth", "pin", "pw"];

/// Keys whose value is a secret even when it is a number (a PIN, a numeric password); under the other denied keys a
/// number is a count (`max_tokens`) and stays.
const NUMERIC_SECRET_KEYS: &[&str] = &[
    "password",
    "passwd",
    "passphrase",
    "senha",
    "secret",
    "segredo",
    "contraseña",
    "contrasena",
    "mot de passe",
    "passwort",
    "kennwort",
    "parola d'ordine",
    "пароль",
    "パスワード",
    "密码",
    "密碼",
    "口令",
];

/// Keys that hold a whole environment: their value is dropped.
const ENV_KEYS: &[&str] = &["env", "environ", "environment", "envp", "variaveis_de_ambiente"];

/// A string longer than this is not sent: its size and hash are.
pub const MAX_STRING_BYTES: usize = 64 * 1024;

pub const REDACTED: &str = "[REDACTED]";

/// In tests, a text with this makes the redactor panic, as a detector bug would.
#[cfg(test)]
pub const TEST_PANIC_MARKER: &str = "__kalango_test_panic_in_redactor__";

/// What a redaction pass did (the span says it: `kalango.redactions`, `kalango.truncated`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub redacted: usize,
    pub omitted_large: usize,
}

pub struct Redactor {
    deny: Vec<String>,
}

impl Default for Redactor {
    fn default() -> Self {
        Redactor::new(DENY_KEYS)
    }
}

fn normalize(key: &str) -> String {
    Folded::new(key).text.chars().filter(|c| !matches!(c, '_' | '-' | ' ' | '.' | '\'')).flat_map(char::to_lowercase).collect()
}

/// The words of a key name, lower case: split at separators and where a lower-case letter meets an upper-case one
/// (`passwordPolicy`, `token_ttl`, `X-Api-Key`).
fn key_words(key: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut prev_lower = false;
    let mut cur = String::new();
    for c in Folded::new(key).text.chars() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !cur.is_empty() {
            words.push(std::mem::take(&mut cur));
        }
        prev_lower = c.is_lowercase();
        cur.extend(c.to_lowercase());
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

/// The last words of a key that name something about a secret, not the secret (`password_policy`, `token_ttl`,
/// `--password-file`, `secret_rotation_days`).
const ABOUT_A_SECRET: &[&str] = &[
    "policy",
    "endpoint",
    "url",
    "uri",
    "path",
    "file",
    "dir",
    "name",
    "type",
    "method",
    "length",
    "len",
    "count",
    "size",
    "min",
    "max",
    "ttl",
    "days",
    "seconds",
    "secs",
    "timeout",
    "expiry",
    "expires",
    "expiration",
    "format",
    "hint",
    "field",
    "prompt",
    "label",
    "mode",
    "scope",
    "scopes",
    "algorithm",
    "kind",
    "enabled",
    "required",
    "rotation",
    "domain",
    "provider",
    "version",
    "regex",
    "pattern",
    "env",
    "var",
    "stdin",
];

/// A key of two or more words whose last word only describes the secret (see [`ABOUT_A_SECRET`]).
fn describes_a_secret(key: &str) -> bool {
    let words = key_words(key);
    words.len() >= 2 && words.last().is_some_and(|w| ABOUT_A_SECRET.contains(&w.as_str()))
}

impl Redactor {
    /// A redactor with its own deny list (the tests build ones with a key left out to show each key is needed).
    pub fn new(deny: &[&str]) -> Redactor {
        Redactor { deny: deny.iter().map(|k| normalize(k)).collect() }
    }

    fn denied(&self, key: &str) -> bool {
        let k = normalize(key);
        !k.is_empty() && !describes_a_secret(key) && (self.deny.iter().any(|d| k.contains(d.as_str())) || suffix_key(key).is_some())
    }

    /// A denied key whose value goes even when it is a number.
    fn numbers_are_secret(&self, key: &str) -> bool {
        let k = normalize(key);
        NUMERIC_SECRET_KEYS.iter().any(|d| k.contains(&normalize(d))) || matches!(suffix_key(key), Some("pwd" | "pass" | "pin" | "pw"))
    }

    /// The value with every secret replaced. Objects: a denied key loses its value (unless it is a number or a
    /// boolean); an environment key loses it whole; strings go through [`Redactor::redact_str`].
    pub fn redact_value(&self, v: &Value, stats: &mut Stats) -> Value {
        match v {
            Value::String(s) => Value::String(self.redact_str(s, stats)),
            Value::Array(a) => Value::Array(a.iter().map(|x| self.redact_value(x, stats)).collect()),
            Value::Object(o) => Value::Object(
                o.iter()
                    .map(|(k, x)| {
                        let kn = normalize(k);
                        if ENV_KEYS.contains(&kn.as_str()) && !x.is_null() {
                            stats.redacted += 1;
                            return (k.clone(), Value::String("[REDACTED:env]".into()));
                        }
                        let kept_number = matches!(x, Value::Number(_)) && !self.numbers_are_secret(k);
                        if self.denied(k) && !kept_number && !matches!(x, Value::Bool(_) | Value::Null) {
                            stats.redacted += 1;
                            return (k.clone(), Value::String(REDACTED.into()));
                        }
                        // a tool call's arguments are a JSON string written by the model: when they do not parse their keys
                        // cannot be seen, so what looks secret is not sent at all
                        if kn == "arguments" {
                            if let Value::String(a) = x {
                                return (k.clone(), Value::String(self.redact_arguments(a, stats)));
                            }
                        }
                        // (a key can be the secret too)
                        (tokens(k, stats), self.redact_value(x, stats))
                    })
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// The text with every secret replaced.
    pub fn redact_str(&self, s: &str, stats: &mut Stats) -> String {
        #[cfg(test)]
        if s.contains(TEST_PANIC_MARKER) {
            panic!("a detector bug (test)");
        }
        if s.len() > MAX_STRING_BYTES {
            stats.omitted_large += 1;
            return large_marker(s);
        }
        // JSON written inside a string (a tool call's arguments, a tool's result): its keys are seen, not just its text
        if let Some(out) = self.structured(s, stats) {
            // (what the structured pass did not flag still goes through the text detectors: a key that is a token, a
            // block inside a value)
            return self.text_pipeline(&out, stats);
        }
        self.text_pipeline(s, stats)
    }

    // ---- the final pass: detection only, over what is about to be sent. It does not trust the first pass's parsing: it
    // decodes every string it finds (to any depth of JSON written inside strings), reads the keys too, and looks at the
    // text with its escapes turned into separators.

    /// How many secrets are still in a value that went through redaction (0 when it is clean).
    pub fn scan_value(&self, v: &Value) -> usize {
        self.scan_value_at(v, 0)
    }

    fn scan_value_at(&self, v: &Value, depth: u8) -> usize {
        let marker = |x: &Value| x.as_str().is_some_and(|t| t.is_empty() || t.starts_with("[REDACTED") || t.starts_with("[omitted"));
        match v {
            Value::String(s) => self.scan_string(s, depth),
            Value::Array(a) => a.iter().map(|x| self.scan_value_at(x, depth)).sum(),
            Value::Object(o) => o
                .iter()
                .map(|(k, x)| {
                    let mut n = self.scan_string(k, depth);
                    let kn = normalize(k);
                    // (an environment key with anything but its marker, or a denied key whose value is not a number, a boolean,
                    // empty or a marker: the first pass missed it)
                    let leaked_env = ENV_KEYS.contains(&kn.as_str()) && !x.is_null();
                    let kept_number = matches!(x, Value::Number(_)) && !self.numbers_are_secret(k);
                    let leaked_secret = self.denied(k) && !kept_number && !matches!(x, Value::Bool(_) | Value::Null);
                    if (leaked_env || leaked_secret) && !marker(x) {
                        n += 1;
                    }
                    n + self.scan_value_at(x, depth)
                })
                .sum(),
            _ => 0,
        }
    }

    fn scan_string(&self, s: &str, depth: u8) -> usize {
        let mut n = self.scan_plain_text(s);
        let t = s.trim();
        if depth < 8 && (t.starts_with('{') || t.starts_with('[') || t.starts_with('"')) {
            if let Ok(v) = serde_json::from_str::<Value>(t) {
                n += self.scan_value_at(&v, depth + 1);
            }
        }
        n
    }

    /// How many secrets the detectors find in a text, with its backslash escapes read as separators (so a secret
    /// after `\n`, or a key written as `\"password\"`, is seen at any depth of escaping).
    pub fn scan_text(&self, text: &str) -> usize {
        // (JSON written as text is redacted by key: its keys are read here too)
        let t = text.trim();
        let json_keys =
            if t.starts_with('{') || t.starts_with('[') { serde_json::from_str::<Value>(t).map_or(0, |v| self.scan_value_at(&v, 1)) } else { 0 };
        self.scan_plain_text(text) + json_keys
    }

    /// [`Redactor::scan_text`] without reading the text as JSON (a string inside JSON is read by [`Redactor::scan_string`]).
    fn scan_plain_text(&self, text: &str) -> usize {
        let mut st = Stats::default();
        if text.contains('\\') {
            self.text_pipeline(&unescape_lossy(text), &mut st);
        }
        self.text_pipeline(text, &mut st);
        st.redacted
    }

    /// The detectors over plain text, one after the other.
    fn text_pipeline(&self, s: &str, stats: &mut Stats) -> String {
        let s = private_keys(s, stats);
        let s = url_credentials(&s, stats);
        let s = self.labelled(&s, stats);
        let s = env_dump(&s, stats);
        tokens(&s, stats)
    }

    /// The secrets that a name points at: `name=value` pairs, sentences ("my password is …"), a label with its value on
    /// the next line, `<password>…</password>`, command-line flags. They are looked for in the text with its disguises
    /// undone ([`Folded`]) and cut from the text as it was written.
    fn labelled(&self, s: &str, stats: &mut Stats) -> String {
        let view = Folded::new(s);
        let t: &str = &view.text;
        let mut ranges = self.key_value_ranges(t);
        ranges.extend(phrase_ranges(t));
        ranges.extend(next_line_ranges(t));
        ranges.extend(self.markup_ranges(t));
        ranges.extend(self.command_ranges(t));
        ranges.extend(cjk_ranges(t));
        let ranges: Vec<(usize, usize)> = ranges.into_iter().map(|(a, b)| (view.origin_of(a), view.origin_of(b))).filter(|(a, b)| a < b).collect();
        replace_ranges(s, &merge_ranges(ranges), REDACTED, stats)
    }

    /// A string that is a JSON object or array: parsed, redacted by key, and written back only when something was
    /// redacted (what holds no secret keeps the way it was written).
    fn structured(&self, s: &str, stats: &mut Stats) -> Option<String> {
        let t = s.trim();
        if !(t.starts_with('{') || t.starts_with('[')) {
            return None;
        }
        let v = serde_json::from_str::<Value>(t).ok()?;
        let mut inner = Stats::default();
        let out = self.redact_value(&v, &mut inner);
        if inner.redacted == 0 && inner.omitted_large == 0 {
            return None;
        }
        stats.redacted += inner.redacted;
        stats.omitted_large += inner.omitted_large;
        Some(out.to_string())
    }

    /// The arguments of a tool call. Parsed (the usual case), they go through the key rules; when they do not parse
    /// they are opaque text: the detectors run, and if anything in it looks secret (a key name, a token) all of it is
    /// omitted.
    fn redact_arguments(&self, s: &str, stats: &mut Stats) -> String {
        // (already omitted by the emitter: its own marker says so)
        if s.starts_with("[omitted: ") {
            return s.to_string();
        }
        if serde_json::from_str::<Value>(s.trim()).is_ok() {
            return self.redact_str(s, stats);
        }
        let mut inner = Stats::default();
        let out = self.redact_str(s, &mut inner);
        if inner.redacted > 0 || self.mentions_denied_key(s) {
            stats.redacted += 1;
            return "[omitted: unparsed arguments]".into();
        }
        stats.omitted_large += inner.omitted_large;
        out
    }

    fn mentions_denied_key(&self, s: &str) -> bool {
        let n = normalize(s);
        self.deny.iter().any(|d| n.contains(d.as_str()))
    }

    /// `password=hunter2`, `"api_key": "abc"`, `Authorization: Bearer abc`, `minha senha de acesso: X`, `password&#61;X`:
    /// the value goes, the name stays.
    fn key_value_ranges(&self, s: &str) -> Vec<(usize, usize)> {
        let b = s.as_bytes();
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        let mut i = 0;
        while i < b.len() {
            let Some((sep_len, colon)) = separator_at(b, i) else {
                i += 1;
                continue;
            };
            // the name before the separator (quotes, spaces and markdown emphasis between them allowed)
            let mut e = i;
            while e > 0 && matches!(b[e - 1], b'"' | b'\'' | b'`' | b' ' | b'\t' | b'\\' | b'*') {
                e -= 1;
            }
            let st =
                s[..e].char_indices().rev().take_while(|(_, c)| c.is_alphanumeric() || matches!(c, '_' | '-' | '.')).last().map_or(e, |(k, _)| k);
            // (a label of several words names a code or a password: its value goes even when it is a number)
            let (strong, by_label) = if st < e && self.denied(&s[st..e]) {
                (password_label(&s[st..e]), false)
            } else if let Some(strong) = label_before(s, e) {
                (strong, true)
            } else {
                i += 1;
                continue;
            };
            let name = &s[st..e];
            // the value after it (markdown emphasis closing the label, `**Password:** X`, is not the value)
            let mut vs = i + sep_len;
            while vs < b.len() && matches!(b[vs], b' ' | b'\t' | b'*') {
                vs += 1;
            }
            // (JSON written as text inside a string has its quotes escaped: `\"password\":\"x\"`)
            let mut after_slashes = vs;
            while after_slashes < b.len() && b[after_slashes] == b'\\' {
                after_slashes += 1;
            }
            let escaped = after_slashes > vs && after_slashes < b.len() && matches!(b[after_slashes], b'"' | b'\'');
            // (how deep the text is written: the backslashes before its quotes, and so before its closing one)
            let depth = if escaped { after_slashes - vs } else { 0 };
            if escaped {
                vs = after_slashes;
            }
            // (a Python string prefix, `f'…'`, `r"…"`, belongs to the quote)
            if vs + 1 < b.len() && matches!(b[vs], b'f' | b'r' | b'b' | b'u' | b'F' | b'R' | b'B') && matches!(b[vs + 1], b'"' | b'\'') {
                vs += 1;
            }
            let quote = if vs < b.len() && matches!(b[vs], b'"' | b'\'' | b'`') { Some(b[vs]) } else { None };
            if quote.is_some() {
                vs += 1;
            }
            let mut ve = vs;
            match quote {
                Some(q) => {
                    // the closing quote is the next one with exactly as many backslashes before it as the opening one had:
                    // an escaped quote inside the value has more (or, at the top level, one)
                    let slashes_before = |at: usize| b[vs..at].iter().rev().take_while(|c| **c == b'\\').count();
                    while ve < b.len() && b[ve] != b'\n' && !(b[ve] == q && slashes_before(ve) == depth) {
                        ve += 1
                    }
                    if ve < b.len() && b[ve] == q {
                        ve -= depth;
                    } else if s[vs..ve].trim().is_empty() || s[vs..ve].starts_with([')', ' ', ',', ';', '\t']) {
                        // (a quote that never closes, followed by nothing like a value, ends the string that holds the
                        // name: `input('Password: ')`)
                        i = vs.max(i + 1);
                        continue;
                    }
                    if interpolation_only(&s[vs..ve]) {
                        i = ve.max(i + 1);
                        continue;
                    }
                }
                None => {
                    ve = value_end(b, vs);
                    if !self.value_shaped(&s[vs..ve], &s[ve..], colon, name) {
                        i = ve.max(i + 1);
                        continue;
                    }
                    // "Authorization: Bearer abc": the scheme is not the secret
                    let word = s[vs..ve].to_ascii_lowercase();
                    if matches!(word.as_str(), "bearer" | "basic" | "token" | "digest") {
                        let mut w = ve;
                        while w < b.len() && matches!(b[w], b' ' | b'\t') {
                            w += 1;
                        }
                        let mut we = w;
                        while we < b.len() && !b[we].is_ascii_whitespace() && !matches!(b[we], b'"' | b'\'' | b',' | b';') {
                            we += 1;
                        }
                        // ("auth: token refreshed" is a log line: after a scheme, a word that does not look like a
                        // credential is not one, except under `Authorization`)
                        let next = &s[w..we.max(w)];
                        if we > w
                            && value_is_literal(next, &s[we..])
                            && (normalize(name).contains("authorization") || credential_shaped(next) || next.contains(['.', '-', '_', '=', '/', '+']))
                        {
                            ve = we;
                        } else {
                            i = ve.max(i + 1);
                            continue;
                        }
                    } else if strong && colon {
                        // a password written after `:` in prose may be several words ("senha: bolo de cenoura"); one that
                        // starts like a sentence about the password ("password: required", "senha: a mesma") is prose
                        match prose_or_passphrase(s, vs, ve, false) {
                            Some(end) => ve = end,
                            None => {
                                i = ve.max(i + 1);
                                continue;
                            }
                        }
                    }
                }
            }
            let value = &s[vs..ve];
            // (a number is a count, `tokens: 120`, except under a password or PIN key)
            let numeric =
                !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit() || c == b'.') && !self.numbers_are_secret(name) && !strong && !by_label;
            if value.is_empty() || numeric || value.starts_with("[REDACTED") || matches!(value, "null" | "true" | "false") {
                i = ve.max(i + 1);
                continue;
            }
            ranges.push((vs, ve));
            i = ve.max(i + 1);
        }
        ranges
    }

    /// Whether an unquoted value after a secret's name has the shape of a secret, not of code or prose: not a call
    /// (`lexer.next_token()`, `await getToken()`), a type (`password: str = …`), a boolean or null, a reference to a
    /// variable or the environment (`os.environ[…]`, `process.env.X`, `${VAR}`, `$VAR`, `env::var(…)`), a structure
    /// (`{ … }`) or a list of upper-case grammar words in prose (`tokens: IDENT, NUMBER`).
    fn value_shaped(&self, value: &str, rest: &str, colon: bool, name: &str) -> bool {
        if value.is_empty() || value.starts_with("[REDACTED") {
            return false;
        }
        if !value_is_literal(value, rest) {
            return false;
        }
        // a YAML tag (`secret: !vault |`); a word under a bare `auth` that is not a credential (`auth=ok`)
        if (value.len() > 1 && value.starts_with('!') && value[1..].bytes().all(|c| c.is_ascii_alphabetic()))
            || (suffix_key(name) == Some("auth") && !credential_shaped(value))
        {
            return false;
        }
        // a type annotation: `password: str = Field(...)`, `token: String,`, `pub secret: SecretString }`
        let after = rest.trim_start_matches([' ', '\t']);
        if colon {
            let base = value.split(['<', '[', '|', '?']).next().unwrap_or(value).trim_start_matches('&');
            let followed_by_assign = after.starts_with('=') && !after.starts_with("==");
            let camel = base.starts_with(|c: char| c.is_ascii_uppercase())
                && base.bytes().all(|c| c.is_ascii_alphanumeric())
                && base.bytes().any(|c| c.is_ascii_lowercase())
                && !base.bytes().any(|c| c.is_ascii_digit());
            if followed_by_assign || KNOWN_TYPES.contains(&base) || (camel && after.starts_with([';', '{', '}', ')', '>', '='])) {
                return false;
            }
        }
        // a list of upper-case words in prose: `tokens: IDENT, NUMBER, STRING`
        let upper_word = |w: &str| {
            !w.is_empty()
                && w.starts_with(|c: char| c.is_ascii_uppercase())
                && w.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
        };
        if upper_word(value)
            && rest.starts_with(',')
            && upper_word(rest[1..].trim_start().split(|c: char| !c.is_ascii_alphanumeric() && c != '_').next().unwrap_or(""))
        {
            return false;
        }
        // a variable that holds the secret, named after it: `password=new_password`, `token: accessToken`, `x = x`
        let ident = value.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') && value.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_');
        if ident && !value.bytes().any(|c| c.is_ascii_digit()) {
            let camel_or_snake = value.contains('_') || value.as_bytes().windows(2).any(|w| w[0].is_ascii_lowercase() && w[1].is_ascii_uppercase());
            let field = name.rsplit('.').next().unwrap_or(name);
            if normalize(value) == normalize(field) || (camel_or_snake && self.deny.iter().any(|d| normalize(value).contains(d.as_str()))) {
                return false;
            }
        }
        true
    }

    /// `<password>X</password>`, `<senha>X</senha>`: the text of an element named after a secret.
    fn markup_ranges(&self, s: &str) -> Vec<(usize, usize)> {
        let mut ranges = Vec::new();
        if !s.contains("</") {
            return ranges;
        }
        let b = s.as_bytes();
        let mut i = 0;
        while let Some(p) = s[i..].find('<').map(|p| p + i) {
            i = p + 1;
            let name_end = s[i..].find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | ':'))).map_or(s.len(), |e| i + e);
            if name_end == i || name_end >= b.len() || !matches!(b[name_end], b'>' | b' ') {
                continue;
            }
            let name = &s[i..name_end];
            let Some(open_end) = s[name_end..].find('>').map(|e| name_end + e + 1) else { break };
            if b[open_end - 2] == b'/' || !self.denied(name) {
                continue;
            }
            let Some(close) = s[open_end..].find("</").map(|e| open_end + e) else { continue };
            let content = &s[open_end..close];
            let t = content.trim();
            if !t.is_empty() && !t.contains('<') && !t.starts_with("[REDACTED") {
                let at = open_end + (content.len() - content.trim_start().len());
                ranges.push((at, at + t.len()));
            }
            i = close;
        }
        ranges
    }

    /// A password given to a command line: `mysql -pX` and `mysql -p X` (and the other MySQL/MariaDB tools), `sshpass -p
    /// X`, `--password X` and every flag named after a secret, `htpasswd -b file user X`, `curl -u user:X`, `echo X | sudo
    /// -S …`, `sudo -S <<< X`.
    fn command_ranges(&self, s: &str) -> Vec<(usize, usize)> {
        const GLUED: &[&str] = &["mysql", "mysqldump", "mysqladmin", "mysqlimport", "mysqlcheck", "mysqlshow", "mariadb", "mariadb-dump"];
        let mut ranges = Vec::new();
        if !s.contains('-') {
            return ranges;
        }
        let mut line_start = 0;
        for line in s.split_inclusive('\n') {
            // (each word's offset from where it is in the line: separators may be several, and several bytes each)
            let words: Vec<(usize, &str)> =
                line.split(char::is_whitespace).filter(|w| !w.is_empty()).map(|w| (w.as_ptr() as usize - line.as_ptr() as usize, w)).collect();
            let command = |w: &str| w.rsplit('/').next().unwrap_or(w).to_string();
            let position = |name: &str| words.iter().position(|(_, w)| command(w) == name);
            let glued = words.iter().any(|(_, w)| GLUED.contains(&command(w).as_str()));
            let sshpass = position("sshpass").is_some();
            let curl = position("curl").is_some();
            let mut push = |at: usize, len: usize| ranges.push((line_start + at, line_start + at + len));
            // a word that can be a flag's value (not another flag, a redirection or a pipe)
            let value_at =
                |n: usize| words.get(n).filter(|(_, v)| !v.starts_with(['-', '<', '>', '|', '&', ';']) && value_is_literal(v, "")).copied();
            for (n, &(at, w)) in words.iter().enumerate() {
                if glued && w.len() > 2 && w.starts_with("-p") && !w.starts_with("-p-") && !w[2..].starts_with('-') {
                    push(at + 2, w.len() - 2);
                } else if (glued && w == "-p") || (sshpass && w == "-p") {
                    // (`mysql -p db` prompts for the password of `db`: the word is a password when another one follows it or
                    // it looks like one)
                    if let Some((vat, v)) = value_at(n + 1) {
                        let more = words.get(n + 2).is_some_and(|(_, x)| !x.starts_with(['<', '>', '|', '&', ';']));
                        if sshpass || more || credential_shaped(v) {
                            push(vat, v.len());
                        }
                    }
                } else if let Some(flag) = w.strip_prefix("--").filter(|f| !f.contains('=') && !f.starts_with("no-") && !f.starts_with("with-")) {
                    if curl && flag == "user" {
                        if let Some((vat, v)) = value_at(n + 1) {
                            if let Some(c) = v.find(':') {
                                push(vat + c + 1, v.len() - c - 1);
                            }
                        }
                    } else if self.denied(flag) {
                        if let Some((vat, v)) = value_at(n + 1) {
                            push(vat, v.len());
                        }
                    }
                } else if curl && (w == "-u" || (w.starts_with("-u") && w.contains(':'))) {
                    let (vat, v) = if w == "-u" { value_at(n + 1).unwrap_or((at, "")) } else { (at + 2, &w[2..]) };
                    if let Some(c) = v.find(':') {
                        push(vat + c + 1, v.len() - c - 1);
                    }
                } else if curl && w.starts_with("--user=") {
                    if let Some(c) = w[7..].find(':') {
                        push(at + 7 + c + 1, w.len() - 7 - c - 1);
                    }
                }
            }
            // htpasswd in batch mode (`-b`): the password is the last of its words (file, user, password; or user,
            // password with `-n`)
            if let Some(h) = position("htpasswd") {
                let flags: String = words[h + 1..].iter().filter(|(_, w)| w.starts_with('-') && !w.starts_with("--")).map(|(_, w)| &w[1..]).collect();
                if flags.contains('b') {
                    let positional: Vec<(usize, &str)> = words[h + 1..].iter().filter(|(_, w)| !w.starts_with('-')).copied().collect();
                    if let Some(&(vat, v)) = positional.get(if flags.contains('n') { 1 } else { 2 }) {
                        push(vat, v.len());
                    }
                }
            }
            // sudo reading its password from standard input
            if let Some(su) = position("sudo") {
                let stdin = words[su + 1..].iter().any(|(_, w)| *w == "--stdin" || (w.starts_with('-') && !w.starts_with("--") && w.contains('S')));
                if stdin {
                    if let Some(h) = words.iter().position(|(_, w)| *w == "<<<") {
                        if let Some(&(vat, v)) = words.get(h + 1) {
                            push(vat, v.len());
                        }
                    }
                    // `echo X | sudo -S …`, `printf '%s\n' X | sudo -S …`
                    if let Some(pipe) = words[..su].iter().rposition(|(_, w)| *w == "|") {
                        if let Some(e) = words[..pipe].iter().rposition(|(_, w)| matches!(command(w).as_str(), "echo" | "printf")) {
                            for &(vat, v) in &words[e + 1..pipe] {
                                if !(v.starts_with('-') && v.len() <= 3) && !v.contains('%') {
                                    push(vat, v.len());
                                }
                            }
                        }
                    }
                }
            }
            line_start += line.len();
        }
        ranges
    }
}

/// The text with every backslash escape (`\n`, `\"`, `\\`, `\u00e9`…) turned into one space.
fn unescape_lossy(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            out.push(b' ');
            i += if b[i + 1] == b'u' { 6 } else { 2 };
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn large_marker(s: &str) -> String {
    format!("[omitted: {} bytes, sha256 {}]", s.len(), sha256_hex(s.as_bytes()))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn replace_ranges(s: &str, ranges: &[(usize, usize)], with: &str, stats: &mut Stats) -> String {
    if ranges.is_empty() {
        return s.to_string();
    }
    // (a range is widened to whole characters: a detector's offset inside a multi-byte character never panics)
    let floor = |mut i: usize| {
        i = i.min(s.len());
        while !s.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let ceil = |mut i: usize| {
        i = i.min(s.len());
        while !s.is_char_boundary(i) {
            i += 1;
        }
        i
    };
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    for &(a, b) in ranges {
        let (a, b) = (floor(a), ceil(b));
        if a < at {
            continue;
        }
        out.push_str(&s[at..a]);
        out.push_str(with);
        at = b;
        stats.redacted += 1;
    }
    out.push_str(&s[at..]);
    out
}

/// `-----BEGIN … PRIVATE KEY-----` up to its END line (or the end of the text).
fn private_keys(s: &str, stats: &mut Stats) -> String {
    const BEGIN: &str = "-----BEGIN ";
    if !s.contains(BEGIN) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find(BEGIN) {
        let after = &rest[p + BEGIN.len()..];
        let Some(label_end) = after.find("-----") else { break };
        let label = &after[..label_end];
        if !label.contains("PRIVATE KEY") {
            out.push_str(&rest[..p + BEGIN.len()]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..p]);
        out.push_str("[REDACTED:private_key]");
        stats.redacted += 1;
        let body = &after[label_end + 5..];
        let end_tag = format!("-----END {label}-----");
        rest = match body.find(&end_tag) {
            Some(e) => &body[e + end_tag.len()..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// `scheme://user:password@host`: the password goes.
fn url_credentials(s: &str, stats: &mut Stats) -> String {
    if !s.contains("://") {
        return s.to_string();
    }
    let mut ranges = Vec::new();
    let mut from = 0;
    while let Some(p) = s[from..].find("://") {
        let start = from + p + 3;
        let end = s[start..].find(|c: char| c == '/' || c.is_whitespace() || c == '"' || c == '\'').map(|e| start + e).unwrap_or(s.len());
        let auth = &s[start..end];
        if let Some(at) = auth.rfind('@') {
            if let Some(colon) = auth[..at].find(':') {
                if colon + 1 < at {
                    ranges.push((start + colon + 1, start + at));
                }
            }
        }
        from = end.max(start);
    }
    replace_ranges(s, &ranges, REDACTED, stats)
}

/// A dump of the environment (three or more `NAME=value` lines): the values go.
fn env_dump(s: &str, stats: &mut Stats) -> String {
    let is_env = |l: &str| {
        let Some((name, value)) = l.split_once('=') else { return false };
        let name = name.strip_prefix("export ").unwrap_or(name);
        name.len() >= 3
            && name.starts_with(|c: char| c.is_ascii_uppercase())
            && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            && !value.is_empty()
            && !value.starts_with("[REDACTED")
    };
    if s.lines().filter(|l| is_env(l)).count() < 3 {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    for l in s.split_inclusive('\n') {
        let body = l.trim_end_matches('\n');
        if is_env(body) {
            let (name, _) = body.split_once('=').unwrap_or((body, ""));
            out.push_str(name);
            out.push_str("=[REDACTED:env]");
            if l.ends_with('\n') {
                out.push('\n');
            }
            stats.redacted += 1;
        } else {
            out.push_str(l);
        }
    }
    out
}

/// The short secret word a key ends with as a whole word (`pwd`, `db_pass`, `X-Auth`, `userPin`): after a separator, at
/// a lower-to-upper case change, or the whole key.
fn suffix_key(key: &str) -> Option<&'static str> {
    let k = key.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    let lower = k.to_ascii_lowercase();
    SUFFIX_KEYS.iter().copied().find(|w| {
        lower.ends_with(w) && {
            let at = k.len() - w.len();
            at == 0 || {
                let (prev, first) = (k.as_bytes()[at - 1], k.as_bytes()[at]);
                !prev.is_ascii_alphanumeric() || (prev.is_ascii_lowercase() && first.is_ascii_uppercase())
            }
        }
    })
}

/// The separator of a `name=value` at `i` and its length, and whether it is a plain `:`: `=`, `:`, `:=`, written for a
/// URL (`%3D`, `%253D`) or as an HTML entity (`&#61;`, `&equals;`). A comparison (`==`, `!=`), an arrow (`=>`) or a
/// path (`std::env`) is not one.
fn separator_at(b: &[u8], i: usize) -> Option<(usize, bool)> {
    let prev = if i > 0 { b[i - 1] } else { 0 };
    match b[i] {
        b'=' if !matches!(b.get(i + 1), Some(b'=' | b'>' | b'~')) && !matches!(prev, b'=' | b'!' | b'<' | b'>') => Some((1, false)),
        b':' if b.get(i + 1) == Some(&b'=') => Some((2, false)),
        b':' if b.get(i + 1) != Some(&b':') && prev != b':' => Some((1, true)),
        b'%' => ["3d", "3a", "253d", "253a"]
            .iter()
            .find(|p| b.get(i + 1..i + 1 + p.len()).is_some_and(|w| w.eq_ignore_ascii_case(p.as_bytes())))
            .map(|p| (1 + p.len(), false)),
        b'&' => ["&#61;", "&#x3d;", "&equals;", "&#58;", "&#x3a;", "&colon;"]
            .iter()
            .find(|p| b.get(i..i + p.len()).is_some_and(|w| w.eq_ignore_ascii_case(p.as_bytes())))
            .map(|p| (p.len(), false)),
        _ => None,
    }
}

/// Where an unquoted value starting at `vs` ends: at a space, a quote, a separator of pairs (`,` `;` `&`), a closing
/// bracket, or an encoded `&`, space or `;` (`%26`, `%20`, `%3B`).
fn value_end(b: &[u8], vs: usize) -> usize {
    // (the bytes themselves: the text comes from outside and a character after `%` may be several bytes)
    let encoded_end = |at: usize| b[at] == b'%' && b.get(at + 1..at + 3).is_some_and(|w| w == b"26" || w == b"20" || w.eq_ignore_ascii_case(b"3B"));
    let mut ve = vs;
    while ve < b.len()
        && !b[ve].is_ascii_whitespace()
        && !matches!(b[ve], b'"' | b'\'' | b'`' | b',' | b';' | b'&' | b')' | b'}' | b'>')
        && !encoded_end(ve)
    {
        ve += 1;
    }
    ve
}

/// Type names that follow `name:` in code (`password: str`, `token: String`).
const KNOWN_TYPES: &[&str] = &[
    "str",
    "String",
    "string",
    "int",
    "float",
    "bool",
    "boolean",
    "bytes",
    "number",
    "any",
    "Any",
    "object",
    "dict",
    "list",
    "Optional",
    "Option",
    "Vec",
    "Box",
    "Arc",
    "Rc",
    "Cow",
    "SecretStr",
    "SecretString",
    "Secret",
    "char",
    "usize",
    "isize",
    "u8",
    "u16",
    "u32",
    "u64",
    "i32",
    "i64",
    "Union",
    "None",
    "unknown",
    "void",
    "Buffer",
    "Uint8Array",
];

/// Words that start code, not a literal value (`await getToken()`, `new Secret(…)`).
const CODE_WORDS: &[&str] = &[
    "await", "new", "yield", "typeof", "lambda", "function", "async", "not", "self", "this", "raise", "return", "throw", "continue", "break",
    "print", "del", "assert",
];

/// Objects whose fields hold configuration, not literals (`os.environ`, `process.env.X`, `settings.SECRET_KEY`).
const CONFIG_OBJECTS: &[&str] = &[
    "os",
    "process",
    "env",
    "std",
    "self",
    "this",
    "config",
    "cfg",
    "conf",
    "settings",
    "args",
    "opts",
    "options",
    "params",
    "request",
    "req",
    "ctx",
    "context",
    "secrets",
    "vault",
    "keyring",
    "system",
    "deno",
    "import",
    "window",
    "props",
    "state",
    "app",
    "input",
    "form",
    "data",
    "credentials",
];

/// Whether a value is written as a literal: not code (a call, an index, `await …`, `new …`), not a reference to a
/// variable, a field of configuration or the environment (`$VAR`, `${VAR}`, `$(cmd)`, `%VAR%`, `{{ x }}`, `<placeholder>`,
/// `os.environ`, `env::var`), not a boolean or null, not a structure (`{ … }`, `[ … ]`) and not a mask (`****`).
fn value_is_literal(value: &str, rest: &str) -> bool {
    // (TypeScript's non-null `!` and an optional `?` after a reference)
    let value = if value.len() > 1 { value.trim_end_matches(['!', '?']) } else { value };
    let lower = value.to_ascii_lowercase();
    if matches!(lower.as_str(), "true" | "false" | "null" | "none" | "nil" | "undefined") || CODE_WORDS.contains(&value) {
        return false;
    }
    if value.starts_with(['{', '[', '(', '<']) || value.starts_with("${") || value.starts_with("$(") || value.contains("{{") || value.contains("::") {
        return false;
    }
    // `$VAR` (a name without digits: `$ecret123` is a password)
    if let Some(var) = value.strip_prefix('$') {
        if !var.is_empty() && var.bytes().all(|c| c.is_ascii_alphabetic() || c == b'_') {
            return false;
        }
    }
    // (`%VAR%`, and a format placeholder: `%s`, `%(name)s`)
    if (value.len() > 2 && value.starts_with('%') && value.ends_with('%'))
        || (value.starts_with('%') && value[1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '('))
    {
        return false;
    }
    if value.len() >= 3 && value.chars().all(|c| matches!(c, '*' | 'x' | 'X' | '•' | '.')) {
        return false;
    }
    // a call or an index: an identifier path right before `(` or `[`
    let ident_path = |p: &str| {
        !p.is_empty()
            && p.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '$')
            && p.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'$' | b'?'))
    };
    if let Some(at) = value.find(['(', '[']) {
        if ident_path(&value[..at]) {
            return false;
        }
    }
    if rest.starts_with(['(', '[']) && ident_path(value) {
        return false;
    }
    // a field of a configuration object: `os.environ`, `process.env.API_KEY`, `settings.SECRET_KEY`
    if let Some((first, more)) = value.split_once('.') {
        if !more.is_empty() && ident_path(value) && CONFIG_OBJECTS.contains(&first.to_ascii_lowercase().as_str()) {
            return false;
        }
    }
    true
}

/// A quoted value that only interpolates something else (`"${DB_PASSWORD}"`, `` `Bearer ${token}` ``, `"{{ secret }}"`).
fn interpolation_only(v: &str) -> bool {
    // the text with every `${…}`, `{{…}}`, `{…}`, `$(…)` and `$NAME` taken out
    let b = v.as_bytes();
    let (mut rest, mut found, mut i) = (String::new(), false, 0);
    while i < b.len() {
        let open = if b[i..].starts_with(b"{{") {
            Some(("}}", 2))
        } else if b[i..].starts_with(b"${") || b[i..].starts_with(b"$(") {
            Some((if b[i + 1] == b'{' { "}" } else { ")" }, 2))
        } else if b[i] == b'{' {
            Some(("}", 1))
        } else {
            None
        };
        if let Some((close, len)) = open {
            if let Some(e) = v[i + len..].find(close) {
                found = true;
                i += len + e + close.len();
                continue;
            }
        }
        if b[i] == b'$' && b.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_') {
            found = true;
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            continue;
        }
        let n = v[i..].chars().next().map_or(1, char::len_utf8);
        rest.push_str(&v[i..i + n]);
        i += n;
    }
    found && rest.split_whitespace().all(|w| matches!(w.to_ascii_lowercase().as_str(), "bearer" | "basic" | "token"))
}

/// The words that name a password (the value after them may be several words, and is a secret even when it is a
/// number). Compared without accents, `_`, `-`, spaces, dots or apostrophes.
const PASSWORD_WORDS: &[&str] = &[
    "password",
    "passwd",
    "passphrase",
    "senha",
    "contrasena",
    "contraseña",
    "passwort",
    "kennwort",
    "motdepasse",
    "paroladordine",
    "пароль",
    "パスワード",
    "密码",
    "密碼",
    "口令",
    "비밀번호",
    "pwd",
    "pw",
    "pass",
];

/// Whether a key or a label names a password (see [`PASSWORD_WORDS`]).
fn password_label(name: &str) -> bool {
    let n = fold_accents(&normalize(name));
    PASSWORD_WORDS.iter().any(|w| n.contains(&fold_accents(w)) && (w.len() > 4 || n == *w || suffix_key(name).is_some()))
}

/// Labels of more than one word (written in the text as words: `mot de passe`, `api key`, `verification code`),
/// compared without spaces or accents.
const MULTI_WORD_LABELS: &[&str] = &[
    "motdepasse",
    "paroladordine",
    "apikey",
    "accesskey",
    "secretkey",
    "accesstoken",
    "authtoken",
    "clientsecret",
    "privatekey",
    "verificationcode",
    "securitycode",
    "codigodeverificacao",
    "codigodeseguranca",
    "onetimecode",
    "2facode",
    "otpcode",
];

/// Prepositions between a label and what it is for ("senha de acesso", "password for the server").
const LABEL_PREPOSITIONS: &[&str] =
    &["de", "do", "da", "dos", "das", "of", "for", "para", "del", "du", "des", "für", "fur", "di", "della", "dello", "pour", "zum", "zur"];

/// A label of several words that ends where a `:` or `=` follows (`mot de passe:`, `api key:`, `minha senha de acesso:`,
/// `password for the server:`): whether there is one, and whether it names a password.
fn label_before(s: &str, end: usize) -> Option<bool> {
    // up to five words before `end`, on its line, separated by spaces only
    let mut words: Vec<&str> = Vec::new();
    let mut e = end;
    while words.len() < 5 {
        let st = s[..e].char_indices().rev().take_while(|(_, c)| c.is_alphanumeric() || matches!(c, '\'' | '-' | '_')).last().map_or(e, |(k, _)| k);
        if st == e {
            break;
        }
        words.push(&s[st..e]);
        let before = s[..st].trim_end_matches([' ', '\t']);
        if before.len() == st {
            break;
        }
        e = before.len();
    }
    if words.len() < 2 {
        return None;
    }
    words.reverse();
    let n = words.len();
    for k in 2..=n {
        let joined = fold_accents(&normalize(&words[n - k..].concat()));
        if MULTI_WORD_LABELS.iter().any(|m| joined.ends_with(m)) {
            return Some(password_label(&joined));
        }
    }
    for k in 3..=n {
        let (label, prep) = (words[n - k], fold_accents(words[n - k + 1]));
        if password_label(label) && normalize(label).len() > 3 && LABEL_PREPOSITIONS.contains(&prep.as_str()) {
            return Some(true);
        }
    }
    None
}

/// Words that start a sentence about a password, not the password ("the password is wrong", "a senha é obrigatória").
/// An adjective among them is prose when the clause ends after it or goes on with prose ("obrigatória para entrar");
/// followed by other words it may be the first word of a passphrase ("correct horse battery staple").
const PROSE_WORDS: &[&str] = &[
    // determiners, pronouns
    "a",
    "o",
    "as",
    "os",
    "um",
    "uma",
    "the",
    "an",
    "sua",
    "minha",
    "seu",
    "meu",
    "your",
    "my",
    "our",
    "isso",
    "essa",
    "esta",
    "este",
    "this",
    "that",
    "it",
    "la",
    "el",
    "los",
    "las",
    "un",
    "una",
    "mi",
    "tu",
    "su",
    "le",
    "les",
    "une",
    "mon",
    "ma",
    "votre",
    "ton",
    "der",
    "die",
    "das",
    "ein",
    "eine",
    "mein",
    "dein",
    "kein",
    "il",
    "lo",
    "mia",
    "mio",
    "tua",
    // adverbs, negations
    "nao",
    "muito",
    "ainda",
    "sempre",
    "nunca",
    "tambem",
    "so",
    "too",
    "not",
    "very",
    "still",
    "never",
    "always",
    "also",
    "only",
    "just",
    "now",
    "agora",
    "no",
    "muy",
    "demasiado",
    "trop",
    "pas",
    "tres",
    "nicht",
    "zu",
    "sehr",
    "troppo",
    "non",
    "molto",
    // prepositions, conjunctions
    "at",
    "in",
    "on",
    "for",
    "to",
    "of",
    "by",
    "with",
    "from",
    "em",
    "de",
    "do",
    "da",
    "para",
    "por",
    "com",
    "sem",
    "and",
    "or",
    "but",
    "e",
    "ou",
    "mas",
    "que",
    "if",
    "se",
    "being",
    "here",
    "there",
    "aqui",
    // verbs that start an instruction
    "see",
    "use",
    "check",
    "enter",
    "type",
    "ask",
    "contact",
    "ver",
    "veja",
    "use",
    "digite",
    "informe",
    "consulte",
    "pergunte",
    "defina",
    "[redacted]",
];

const PROSE_ADJECTIVES: &[&str] = &[
    "obrigatoria",
    "obrigatorio",
    "forte",
    "fraca",
    "fraco",
    "segura",
    "seguro",
    "errada",
    "errado",
    "incorreta",
    "incorreto",
    "invalida",
    "invalido",
    "valida",
    "valido",
    "nova",
    "novo",
    "antiga",
    "antigo",
    "curta",
    "longa",
    "necessaria",
    "pessoal",
    "secreta",
    "vazia",
    "igual",
    "diferente",
    "outra",
    "mesma",
    "trocada",
    "alterada",
    "expirada",
    "wrong",
    "incorrect",
    "invalid",
    "valid",
    "required",
    "weak",
    "strong",
    "correct",
    "expired",
    "empty",
    "blank",
    "set",
    "missing",
    "needed",
    "mandatory",
    "optional",
    "opcional",
    "same",
    "different",
    "changed",
    "case",
    "sensitive",
    "reset",
    "secret",
    "new",
    "old",
    "short",
    "long",
    "personal",
    "private",
    "unique",
    "hidden",
    "obligatoire",
    "requis",
    "falsch",
    "richtig",
    "erforderlich",
    "sbagliata",
    "corretta",
    "obbligatoria",
    "obligatoria",
    "requerida",
    "incorrecta",
    "correcta",
    "debil",
    "fuerte",
    "segura",
    "kurz",
    "lang",
    "corta",
    "courte",
    "longue",
];

/// Words that end the thought after an adjective: what comes after them is prose, not a passphrase.
const PROSE_FOLLOWERS: &[&str] =
    &["yesterday", "today", "again", "ontem", "hoje", "agora", "now", "please", "por favor", "try", "tente", "aqui", "here"];

/// Whether an all-letters word looks like a participle ("changed", "alterada", "enviada"): prose after "is" or "é".
fn participle(w: &str) -> bool {
    w.chars().count() >= 5 && ["ed", "ado", "ada", "ido", "ida", "ados", "adas", "idos", "idas"].iter().any(|e| w.ends_with(e))
}

/// The value after a password's name that starts at `vs` (its first word ends at `ve`): `None` when the words read as a
/// sentence about the password, else where the password ends. A first word with a digit or a symbol is the password
/// alone; a first word of letters starts a passphrase that runs to the end of the clause (a line, a sentence, a `,`
/// or `;`, a closing quote or bracket, or the next `name:`).
fn prose_or_passphrase(s: &str, vs: usize, ve: usize, lone_adjective_is_prose: bool) -> Option<usize> {
    let first = s[vs..ve].trim_end_matches(['.', '!', '?', ':', ',', ';', ')', '"', '\'', '»', '”']);
    if first.is_empty() {
        return None;
    }
    if !first.chars().all(char::is_alphabetic) {
        return Some(vs + first.len());
    }
    let word = fold_accents(first);
    if PROSE_WORDS.contains(&word.as_str()) {
        return None;
    }
    let ends_clause = first.len() < s[vs..ve].len();
    let end = if ends_clause { vs + first.len() } else { clause_end(s, vs + first.len()) };
    if PROSE_ADJECTIVES.contains(&word.as_str()) || participle(&word) {
        // the next word of the clause decides: prose after it, or nothing ("the password is wrong.") when a lone
        // adjective reads as prose
        let next = s[vs + first.len()..end].split_whitespace().next().map(|w| fold_accents(w.trim_end_matches(['.', '!', '?'])));
        let prose_next = next.as_ref().is_some_and(|n| {
            PROSE_WORDS.contains(&n.as_str()) || PROSE_ADJECTIVES.contains(&n.as_str()) || PROSE_FOLLOWERS.contains(&n.as_str()) || participle(n)
        });
        if prose_next || (lone_adjective_is_prose && next.is_none()) {
            return None;
        }
    }
    Some(end)
}

/// Where the clause that goes on at `from` ends: a line break, the end of a sentence, `,` `;` `|`, a quote or a closing
/// bracket, or a word that starts another pair (`host:`, `user=ana`). At most 16 more words.
fn clause_end(s: &str, from: usize) -> usize {
    let b = s.as_bytes();
    let mut end = from;
    for _ in 0..16 {
        let mut w = end;
        while w < b.len() && matches!(b[w], b' ' | b'\t') {
            w += 1;
        }
        if w == end || w >= b.len() {
            break;
        }
        let mut we = w;
        while we < b.len()
            && !b[we].is_ascii_whitespace()
            && !matches!(b[we], b'"' | b'\'' | b'`' | b',' | b';' | b'|' | b')' | b'}' | b']' | b'>' | b'&' | b'(')
        {
            we += 1;
        }
        if we == w {
            break;
        }
        let word = &s[w..we];
        if word.ends_with(':') || word.contains('=') {
            break;
        }
        let trimmed = word.trim_end_matches(['.', '!', '?']);
        end = w + trimmed.len();
        if trimmed.len() < word.len() {
            break;
        }
    }
    end
}

/// A word that looks like a credential, not like prose: four or more characters with a digit (`banana2024`, `482913`),
/// or with an upper-case letter after a lower-case one (`HunterTwo`), or letters with a symbol (`hunter!x`).
fn credential_shaped(w: &str) -> bool {
    let w = w.trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '.' | ',' | ';' | '!' | '?' | ')' | '('));
    let has = |f: fn(char) -> bool| w.chars().any(f);
    w.chars().count() >= 4
        && (has(|c| c.is_ascii_digit())
            || w.as_bytes().windows(2).any(|p| p[0].is_ascii_lowercase() && p[1].is_ascii_uppercase())
            || (has(char::is_alphabetic) && has(|c| matches!(c, '!' | '@' | '#' | '$' | '%' | '^' | '&' | '*' | '+' | '=' | '~' | '_'))))
}

/// Words that name a password in a sentence ("a senha é …", "my password is …", "mot de passe est …"); several words
/// are compared without accents, one space between them.
const PHRASE_KEYS: &[&str] = &[
    "senha",
    "password",
    "passwd",
    "passphrase",
    "pwd",
    "pw",
    "pin",
    "contraseña",
    "contrasena",
    "passwort",
    "kennwort",
    "пароль",
    "mot de passe",
    "parola d'ordine",
];
/// Words that name a secret that is a credential only when the value looks like one ("o token é Hunt3r", "my OTP is
/// 482913"; "the token is rotated daily" is prose).
const PHRASE_WEAK_KEYS: &[&str] = &[
    "token",
    "secret",
    "segredo",
    "clave",
    "chave",
    "api key",
    "access key",
    "secret key",
    "access token",
    "2fa",
    "otp",
    "totp",
    "mfa",
    "verification code",
    "security code",
    "codigo de verificacao",
    "codigo de seguranca",
    "one-time code",
    "auth code",
];
/// The verb between the name and the value.
const PHRASE_VERBS: &[&str] =
    &["é", "eh", "era", "será", "sera", "fica", "is", "was", "=", ":", "es", "est", "ist", "lautet", "war", "è", "—", "это"];

/// "a senha é hunter2", "Minha senha do wifi é: X", "my password is correct horse battery staple", "mi contraseña es X",
/// "meu código 2FA é 482913": the value after the verb goes (the name may be followed by up to three words, "do wifi",
/// "for the server", before the verb).
fn phrase_ranges(s: &str) -> Vec<(usize, usize)> {
    // (word, start, end) for every run of non-space characters
    let mut words: Vec<(&str, usize, usize)> = Vec::new();
    let mut start = None;
    for (i, c) in s.char_indices().chain(std::iter::once((s.len(), ' '))) {
        match (c.is_whitespace(), start) {
            (true, Some(st)) => {
                words.push((&s[st..i], st, i));
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    let bare = |w: &str| {
        w.trim_matches(|c: char| matches!(c, ',' | ';' | '.' | '!' | '?' | '"' | '\'' | '`' | '(' | ')' | '*')).trim_end_matches(':').to_lowercase()
    };
    let clean = |w: &str| fold_accents(&bare(w));
    let mut ranges = Vec::new();
    // (each word cleaned once, each name split once: one pass over the text)
    let cleaned: Vec<String> = words.iter().map(|w| clean(w.0)).collect();
    let names: Vec<(Vec<String>, bool, bool)> = [(PHRASE_KEYS, true), (PHRASE_WEAK_KEYS, false)]
        .iter()
        .flat_map(|(keys, strong)| {
            keys.iter().map(move |key| (key.split(' ').map(fold_accents).collect(), *strong, key.starts_with(|c: char| !c.is_ascii())))
        })
        .collect();
    let mut k = 0;
    while k < words.len() {
        // the first name that starts here
        let name = names.iter().find_map(|(parts, strong, non_latin)| {
            let n = parts.len();
            (cleaned.get(k..k + n)? == parts.as_slice()).then_some((n, *strong, *non_latin))
        });
        let Some((n, strong, non_latin)) = name else {
            k += 1;
            continue;
        };
        let after_name = k + n;
        // (a name that ends its clause, `{ password, …rest }`, is not followed by its value)
        if words[after_name - 1].0.ends_with([',', ';', ')']) {
            k += 1;
            continue;
        }
        // a verb within four words; a name in Cyrillic needs none ("мой пароль qwerty1")
        let verb = (after_name..(after_name + 4).min(words.len())).find(|&j| {
            let w = words[j].0.trim_end_matches([':', ',']).to_lowercase();
            PHRASE_VERBS.contains(&w.as_str()) || PHRASE_VERBS.contains(&words[j].0)
        });
        let mut j = match verb {
            Some(v) => v + 1,
            None if non_latin && words.get(after_name).is_some_and(|w| credential_shaped(w.0)) => after_name,
            None => {
                k += 1;
                continue;
            }
        };
        if j < words.len() && matches!(words[j].0, ":" | "—" | "-") {
            j += 1;
        }
        if j >= words.len() {
            break;
        }
        let (raw, ws, we) = words[j];
        // a quoted value runs to its closing quote ("my password is 'correct horse' ok")
        if let Some(q) = raw.chars().next().filter(|c| matches!(c, '"' | '\'' | '`' | '“' | '«')) {
            let close = match q {
                '“' => '”',
                '«' => '»',
                q => q,
            };
            let inner = ws + q.len_utf8();
            if let Some(c) = s[inner..].find(close).map(|c| inner + c) {
                if c > inner && !s[inner..c].contains('\n') && !s[inner..c].starts_with("[REDACTED") && (strong || credential_shaped(&s[inner..c])) {
                    ranges.push((inner, c));
                }
                k = j + 1;
                continue;
            }
        }
        let value = raw.trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '(' | ')' | '«' | '»' | '“' | '”'));
        let value = value.trim_end_matches([',', ';', '.', ')', '!', '?']);
        let offset = raw.find(value).unwrap_or(0);
        let (vs, ve) = (ws + offset, ws + offset + value.len());
        if value.is_empty() || value.starts_with("[REDACTED") || !value_is_literal(value, &s[ve.min(we)..]) || (!strong && !credential_shaped(value))
        {
            k = j;
            continue;
        }
        let end = if strong { prose_or_passphrase(s, vs, we, true) } else { Some(ve) };
        match end {
            Some(end) => {
                ranges.push((vs, end));
                k = j + 1;
            }
            None => k = j,
        }
    }
    ranges
}

/// A label alone on its line ("Senha", "**Password:**") with a credential alone on the next line: that line goes.
fn next_line_ranges(s: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    if !s.contains('\n') {
        return ranges;
    }
    let mut lines: Vec<(usize, &str)> = Vec::new();
    let mut at = 0;
    for l in s.split_inclusive('\n') {
        lines.push((at, l));
        at += l.len();
    }
    for pair in lines.windows(2) {
        let label = pair[0].1.trim().trim_matches(|c: char| matches!(c, '*' | '#' | '-' | '>' | '_' | ':' | ' ' | '\t')).trim();
        let said = fold_accents(label);
        let is_label = PHRASE_KEYS.iter().any(|k| fold_accents(k) == said)
            || (said.split(' ').count() <= 4
                && said.split(' ').next().is_some_and(|w| PHRASE_KEYS.contains(&w) && w.len() > 3)
                && said.split(' ').nth(1).is_some_and(|w| LABEL_PREPOSITIONS.contains(&w)));
        if !is_label {
            continue;
        }
        let (next_at, next) = pair[1];
        let t = next.trim();
        if t.is_empty() || t.contains(char::is_whitespace) || t.starts_with("[REDACTED") || !credential_shaped(t) {
            continue;
        }
        let from = next_at + (next.len() - next.trim_start().len());
        ranges.push((from, from + t.len()));
    }
    ranges
}

/// "密码：X", "密码是X", "パスワードはXです": in Chinese, Japanese and Korean no space separates the label from the value;
/// the value is the run of ASCII after the label and its particle.
fn cjk_ranges(s: &str) -> Vec<(usize, usize)> {
    const LABELS: &[&str] = &["密码", "密碼", "口令", "パスワード", "비밀번호"];
    let mut ranges = Vec::new();
    if s.is_ascii() {
        return ranges;
    }
    for label in LABELS {
        let mut from = 0;
        while let Some(p) = s[from..].find(label).map(|p| p + from) {
            from = p + label.len();
            let rest = &s[from..];
            let skip: usize = rest
                .chars()
                .take_while(|c| matches!(c, ' ' | '\t' | ':' | '=' | '是' | '为' | '為' | 'は' | 'が' | '：' | '는' | '은'))
                .map(char::len_utf8)
                .sum();
            let vs = from + skip;
            let len: usize = s[vs..]
                .chars()
                .take_while(|c| c.is_ascii_graphic() && !matches!(c, '"' | '\'' | '`' | ')' | '}' | ']' | '>' | ',' | ';'))
                .map(char::len_utf8)
                .sum();
            let value = s[vs..vs + len].trim_end_matches(['.', ',', ';', '!', '?', ')']);
            if !value.is_empty() && !value.starts_with("[REDACTED") {
                ranges.push((vs, vs + value.len()));
            }
        }
    }
    ranges
}

/// Ranges sorted, the ones that overlap joined into one.
fn merge_ranges(mut ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    ranges.sort();
    let mut out: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for (a, b) in ranges {
        match out.last_mut() {
            Some(last) if a < last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// Lower case without the accents of Portuguese and Spanish (`obrigatória` → `obrigatoria`, `contraseña` →
/// `contrasena`).
fn fold_accents(w: &str) -> String {
    w.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' => 'a',
            'é' | 'ê' => 'e',
            'í' => 'i',
            'ó' | 'ô' | 'õ' => 'o',
            'ú' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}

/// A format character that draws nothing (zero-width space, joiners, direction marks, BOM, soft hyphen, variation
/// selectors): removed before names are compared, so `pass\u{200b}word` is `password`.
fn invisible(c: char) -> bool {
    matches!(c, '\u{ad}' | '\u{34f}' | '\u{61c}' | '\u{115f}' | '\u{1160}' | '\u{17b4}' | '\u{17b5}' | '\u{180b}'..='\u{180f}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206f}' | '\u{3164}' | '\u{fe00}'..='\u{fe0f}' | '\u{feff}' | '\u{ffa0}')
}

/// Full-width ASCII (`ｐａｓｓ：`, U+FF01 to U+FF5E) and the Unicode spaces, as plain ASCII.
fn plain_width(c: char) -> Option<char> {
    match c {
        '\u{ff01}'..='\u{ff5e}' => char::from_u32(c as u32 - 0xfee0),
        '\u{a0}' | '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}' => Some(' '),
        _ => None,
    }
}

/// The Cyrillic and Greek letters drawn like Latin ones.
fn homoglyph(c: char) -> Option<char> {
    Some(match c {
        'а' | 'α' => 'a',
        'е' | 'ε' => 'e',
        'о' | 'ο' => 'o',
        'р' | 'ρ' => 'p',
        'с' => 'c',
        'у' => 'y',
        'х' | 'χ' => 'x',
        'і' | 'ι' => 'i',
        'ј' => 'j',
        'ѕ' => 's',
        'ԁ' => 'd',
        'һ' => 'h',
        'ԛ' => 'q',
        'ԝ' => 'w',
        'к' | 'κ' => 'k',
        'ν' => 'v',
        'υ' => 'u',
        'τ' => 't',
        'А' | 'Α' => 'A',
        'В' | 'Β' => 'B',
        'Е' | 'Ε' => 'E',
        'К' | 'Κ' => 'K',
        'М' | 'Μ' => 'M',
        'Н' | 'Η' => 'H',
        'О' | 'Ο' => 'O',
        'Р' | 'Ρ' => 'P',
        'С' => 'C',
        'Т' | 'Τ' => 'T',
        'Х' | 'Χ' => 'X',
        'І' | 'Ι' => 'I',
        'Ј' => 'J',
        'Ѕ' => 'S',
        'У' | 'Υ' => 'Y',
        'Ζ' => 'Z',
        'Ν' => 'N',
        _ => return None,
    })
}

/// The text with its disguises undone, for finding the names of secrets: invisible format characters removed, full-width
/// ASCII and Unicode spaces made plain, and the Cyrillic and Greek letters drawn like Latin ones made Latin inside a word
/// that mixes them with Latin letters or is made only of them (`pаssword` with a Cyrillic `а`; `пароль` stays). Each byte
/// remembers where it came from, so a range found in the folded text is cut from the text as it was written.
struct Folded<'a> {
    text: std::borrow::Cow<'a, str>,
    /// for each byte of `text`, and one past its end, the byte of the original it came from (empty: the same text)
    origin: Vec<usize>,
}

impl<'a> Folded<'a> {
    fn new(s: &'a str) -> Folded<'a> {
        if s.is_ascii() || !s.chars().any(|c| invisible(c) || plain_width(c).is_some() || homoglyph(c).is_some()) {
            return Folded { text: std::borrow::Cow::Borrowed(s), origin: Vec::new() };
        }
        // (offset, length in the original, the character with its width made plain)
        let chars: Vec<(usize, usize, char)> =
            s.char_indices().filter(|(_, c)| !invisible(*c)).map(|(i, c)| (i, c.len_utf8(), plain_width(c).unwrap_or(c))).collect();
        let mut text = String::with_capacity(s.len());
        let mut origin = Vec::with_capacity(s.len() + 1);
        let mut w = 0;
        while w < chars.len() {
            // a word: a run of letters and digits (any other character is a word of its own)
            let mut e = w;
            while e < chars.len() && chars[e].2.is_alphanumeric() {
                e += 1;
            }
            let e = e.max(w + 1);
            let word = &chars[w..e];
            let latin = word.iter().any(|x| x.2.is_ascii_alphabetic()) || word.iter().all(|x| homoglyph(x.2).is_some());
            for &(at, len, c) in word {
                let c = if latin { homoglyph(c).unwrap_or(c) } else { c };
                let n = c.len_utf8();
                // (a character kept as it was keeps its bytes' offsets; one that changed points at its start)
                origin.extend((0..n).map(|k| if n == len { at + k } else { at }));
                text.push(c);
            }
            w = e;
        }
        origin.push(s.len());
        Folded { text: std::borrow::Cow::Owned(text), origin }
    }

    /// Where a byte offset of the folded text is in the original.
    fn origin_of(&self, i: usize) -> usize {
        if self.origin.is_empty() {
            i
        } else {
            self.origin[i.min(self.origin.len() - 1)]
        }
    }
}

fn is_token_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'+')
}

fn shannon(t: &str) -> f64 {
    let mut counts = [0usize; 256];
    for b in t.bytes() {
        counts[b as usize] += 1;
    }
    let n = t.len() as f64;
    counts
        .iter()
        .filter(|c| **c > 0)
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum()
}

fn looks_like_uuid(t: &str) -> bool {
    t.len() == 36 && t.bytes().enumerate().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { c == b'-' } else { c.is_ascii_hexdigit() })
}

/// A run of text that is a credential: a known prefix, or long, mixed and random-looking.
pub fn is_secret_token(t: &str) -> bool {
    let t = t.trim_matches(|c| c == '.' || c == '-');
    if t.len() < 16 {
        return false;
    }
    const PREFIXES: &[(&str, usize)] = &[
        ("ghp_", 24),
        ("gho_", 24),
        ("ghu_", 24),
        ("ghs_", 24),
        ("ghr_", 24),
        ("github_pat_", 30),
        ("xoxb-", 20),
        ("xoxp-", 20),
        ("xoxa-", 20),
        ("xoxs-", 20),
        ("xoxr-", 20),
        ("sk-", 20),
        ("sk_live_", 20),
        ("pk_live_", 20),
        ("rk_live_", 20),
        ("hf_", 24),
        ("glpat-", 20),
        ("npm_", 24),
        ("AIza", 35),
    ];
    if PREFIXES.iter().any(|(p, min)| t.starts_with(p) && t.len() >= *min) {
        return true;
    }
    if (t.starts_with("AKIA") || t.starts_with("ASIA")) && t.len() == 20 && t.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
        return true;
    }
    // a JWT: three dot-separated base64url parts, the first one a JSON header
    if t.starts_with("eyJ") && t.matches('.').count() == 2 && t.len() >= 30 {
        return true;
    }
    if looks_like_uuid(t) || t.bytes().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    let has_digit = t.bytes().any(|c| c.is_ascii_digit());
    let has_alpha = t.bytes().any(|c| c.is_ascii_alphabetic());
    if t.len() < 24 {
        // 16 to 23 characters: random-looking only when it mixes all three kinds and keeps switching between them (an
        // identifier like `HttpClient2Builder` switches a few times; a random key nearly every character)
        let has_upper = t.bytes().any(|c| c.is_ascii_uppercase());
        let has_lower = t.bytes().any(|c| c.is_ascii_lowercase());
        let class = |c: u8| {
            if c.is_ascii_digit() {
                0
            } else if c.is_ascii_uppercase() {
                1
            } else if c.is_ascii_lowercase() {
                2
            } else {
                3
            }
        };
        let switches = t.as_bytes().windows(2).filter(|w| class(w[0]) != class(w[1])).count();
        return has_upper && has_lower && has_digit && switches * 2 >= t.len() && shannon(t) >= 3.5;
    }
    has_digit && has_alpha && shannon(t) >= 4.0
}

/// A run of base64 with `/` or `+` in it that is a key (an AWS secret access key: 40 characters, `/` inside): mixed,
/// random-looking, not a path (a path starts with `/` or has short words between its slashes).
fn is_secret_base64(t: &str) -> bool {
    let t = t.trim_end_matches('=');
    if t.len() < 30 || t.starts_with('/') || !(t.contains('/') || t.contains('+')) || t.matches('/').count() > 4 {
        return false;
    }
    let segments_long = t.split(['/', '+']).all(|p| p.len() >= 4);
    let has = |f: fn(&u8) -> bool| t.as_bytes().iter().any(f);
    segments_long && has(u8::is_ascii_uppercase) && has(u8::is_ascii_lowercase) && has(u8::is_ascii_digit) && shannon(t) >= 4.0
}

/// A known prefix written too short: the start of a token that a space split (`ghp_ABC… DEF…`).
fn known_prefix_too_short(t: &str) -> bool {
    ["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_", "xoxb-", "xoxp-", "sk-", "sk_live_", "hf_", "glpat-", "npm_", "AIza", "AKIA", "ASIA"]
        .iter()
        .any(|p| t.starts_with(p) && t.len() > p.len())
}

/// Known-prefix and high-entropy tokens inside free text.
fn tokens(s: &str, stats: &mut Stats) -> String {
    let b = s.as_bytes();
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !is_token_char(b[i]) {
            i += 1;
            continue;
        }
        let st = i;
        while i < b.len() && is_token_char(b[i]) {
            i += 1;
        }
        if i - st >= 16 && is_secret_token(&s[st..i]) {
            ranges.push((st, i));
        } else if known_prefix_too_short(&s[st..i]) && i + 1 < b.len() && b[i] == b' ' && b[i + 1].is_ascii_alphanumeric() {
            // the rest of a split token after one space: the two together are the secret
            let mut e = i + 1;
            while e < b.len() && is_token_char(b[e]) {
                e += 1;
            }
            let joined = format!("{}{}", &s[st..i], &s[i + 1..e]);
            if e - (i + 1) >= 8 && is_secret_token(&joined) {
                ranges.push((st, e));
                i = e;
            }
        }
    }
    // keys written in base64 with `/` or `+` (the run above stops at them)
    let mut i = 0;
    while i < b.len() {
        if !(b[i].is_ascii_alphanumeric() || matches!(b[i], b'/' | b'+' | b'=')) {
            i += 1;
            continue;
        }
        let st = i;
        while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'/' | b'+' | b'=')) {
            i += 1;
        }
        if is_secret_base64(&s[st..i]) {
            // (a piece of it may already be marked by the run above: the whole key goes, as one)
            let (mut a, mut e) = (st, i);
            ranges.retain(|&(x, y)| {
                let overlaps = x < e && a < y;
                if overlaps {
                    a = a.min(x);
                    e = e.max(y);
                }
                !overlaps
            });
            ranges.push((a, e));
        }
    }
    ranges.sort();
    replace_ranges(s, &ranges, REDACTED, stats)
}

/// A fake PEM block for the tests, built at run time: no literal in the source looks like a key to a secret scanner.
#[cfg(test)]
pub(crate) fn test_pem(kind: &str, body: &str, closed: bool) -> String {
    let (begin, end, dashes) = (concat!("-----", "BEGIN "), concat!("-----", "END "), concat!("--", "---"));
    let mut pem = format!("{begin}{kind} PRIVATE KEY{dashes}\n{body}");
    if closed {
        pem.push_str(&format!("\n{end}{kind} PRIVATE KEY{dashes}"));
    }
    pem
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn red(s: &str) -> String {
        Redactor::default().redact_str(s, &mut Stats::default())
    }

    #[test]
    fn deny_list_has_the_required_keys() {
        for k in ["password", "token", "secret", "authorization", "api_key", "passphrase", "private_key", "cookie"] {
            assert!(DENY_KEYS.contains(&k), "{k} must stay in the deny list");
        }
    }

    #[test]
    fn every_denied_key_is_needed_mutating_the_list_leaks() {
        // for each key: a redactor with the key removed lets the value through; the default does not. A change to
        // DENY_KEYS (a key dropped or misspelt) breaks the second assertion.
        for key in ["password", "token", "secret", "authorization", "api_key", "passphrase", "private_key", "cookie"] {
            let payload = json!({ key: "plain-value-123", "other": { key.to_uppercase(): "second-value" } });
            let mutated: Vec<&str> =
                DENY_KEYS.iter().copied().filter(|k| !normalize(key).contains(&normalize(k)) && !normalize(k).contains(&normalize(key))).collect();
            let leaked = Redactor::new(&mutated).redact_value(&payload, &mut Stats::default());
            assert_eq!(leaked[key], "plain-value-123", "without {key} the value must leak (the test is sensitive to the list)");
            let kept = Redactor::default().redact_value(&payload, &mut Stats::default());
            assert_eq!(kept[key], REDACTED, "{key}");
            assert_eq!(kept["other"][key.to_uppercase()], REDACTED, "{key} upper case");
        }
    }

    #[test]
    fn key_names_match_in_every_spelling() {
        let v = json!({"X-Api-Key": "a", "githubToken": "b", "DB_PASSWORD": "c", "Set-Cookie": "d", "Authorization": "Bearer e", "private-key": "f", "max_tokens": 512, "ok": "fine"});
        let r = Redactor::default().redact_value(&v, &mut Stats::default());
        for k in ["X-Api-Key", "githubToken", "DB_PASSWORD", "Set-Cookie", "Authorization", "private-key"] {
            assert_eq!(r[k], REDACTED, "{k}");
        }
        assert_eq!(r["max_tokens"], 512, "numbers under a denied-looking key stay");
        assert_eq!(r["ok"], "fine");
    }

    #[test]
    fn known_prefixes_go() {
        // made-up values, put together at run time so the source never holds a string that looks like a real
        // credential (GitHub push protection and secret scanners would flag it)
        let fakes: Vec<String> = [
            ("ghp", "_abcdefghijklmnopqrstuvwxyz0123456789"),
            ("gho", "_abcdefghijklmnopqrstuvwxyz0123456789"),
            ("xox", "b-1234567890-abcdefghijklmnop"),
            ("sk", "-abcdefghijklmnopqrstuvwxyz123456"),
            ("AKIA", "ABCDEFGHIJKLMNOP"),
            ("hf", "_abcdefghijklmnopqrstuvwxyzABCD"),
            ("eyJhbGciOiJIUzI1NiJ9", ".eyJzdWIiOiIxMjM0NTYifQ.abcdefghijklmnop"),
        ]
        .iter()
        .map(|(a, b)| format!("{a}{b}"))
        .collect();
        for t in &fakes {
            let t = t.as_str();
            let out = red(&format!("use {t} here"));
            assert!(!out.contains(t), "{t} -> {out}");
            assert!(out.contains(REDACTED));
        }
    }

    #[test]
    fn private_key_blocks_go_whole() {
        let pem = format!("before\n{}\nafter", test_pem("OPENSSH", "b3BlbnNzaC1rZXktdjEAAAAA\nAAAAB3NzaC1", true));
        let out = red(&pem);
        assert!(!out.contains("b3BlbnNzaC1"), "{out}");
        assert!(out.starts_with("before\n") && out.ends_with("\nafter"), "{out}");
        // a public key or a certificate is not a secret
        assert!(red(&format!("{}CERTIFICATE{}\nMIIB\n{}CERTIFICATE{}", concat!("-----", "BEGIN "), "-----", concat!("-----", "END "), "-----"))
            .contains("MIIB"));
        // a key with no END line: to the end of the text
        assert!(!red(&test_pem("RSA", "MIIEowIBAAKCAQEA", false)).contains("MIIEow"));
    }

    #[test]
    fn key_value_text_and_headers() {
        assert_eq!(red("password=hunter2 and more"), "password=[REDACTED] and more");
        assert_eq!(red("Authorization: Bearer abc.def-ghi"), "Authorization: [REDACTED]");
        assert!(!red("curl -H 'Authorization: Bearer zzz999' https://x").contains("zzz999"));
        assert!(!red(r#"{"api_key": "sekret-value", "n": 1}"#).contains("sekret-value"));
        assert!(!red("export GITHUB_TOKEN=whatever1").contains("whatever1"));
        assert!(!red("cookie: sessionid=abc123; other=1").contains("abc123"));
        assert!(!red("postgres://admin:s3cretpw@db.local/app").contains("s3cretpw"));
        // numbers and prose are left alone
        assert_eq!(red("tokens: 120 used"), "tokens: 120 used");
        assert_eq!(red("the token is rotated daily"), "the token is rotated daily");
    }

    #[test]
    fn environment_dumps_and_env_keys() {
        let dump = "HOME=/home/ana\nPATH=/usr/bin\nLANG=pt_BR.UTF-8\nEDITOR=vim\n";
        let out = red(dump);
        assert!(!out.contains("/home/ana") && !out.contains("vim"), "{out}");
        assert!(out.contains("HOME=[REDACTED:env]"));
        // one assignment in a command is not a dump
        assert_eq!(red("RUST_LOG=debug cargo test"), "RUST_LOG=debug cargo test");
        let v = Redactor::default().redact_value(&json!({"env": {"A": "1"}, "environment": ["B=2"], "cwd": "/tmp"}), &mut Stats::default());
        assert_eq!(v["env"], "[REDACTED:env]");
        assert_eq!(v["environment"], "[REDACTED:env]");
        assert_eq!(v["cwd"], "/tmp");
    }

    #[test]
    fn high_entropy_tokens_go_but_ordinary_text_stays() {
        assert!(!red("key Zx9Qm3Lp7Vb2Nc8Rt5Yw1Ke4Hd6Gf0Js").contains("Zx9Qm3"));
        let ok = "the file /home/ana/Documents/relatorio-trimestral-final.pdf has sha256 e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855 and id 0af7651916cd43dd8448eb211c80319c uuid 123e4567-e89b-12d3-a456-426614174000 name kalango_agente_motor_turn_state";
        assert_eq!(red(ok), ok);
    }

    #[test]
    fn large_strings_become_size_and_hash() {
        let big = "a".repeat(MAX_STRING_BYTES + 1);
        let mut st = Stats::default();
        let out = Redactor::default().redact_str(&big, &mut st);
        assert!(out.starts_with("[omitted: 65537 bytes, sha256 "), "{out}");
        assert_eq!(st.omitted_large, 1);
        let exact = "b".repeat(MAX_STRING_BYTES);
        assert_eq!(red(&exact), exact);
    }

    #[test]
    fn json_inside_a_string_is_redacted_by_key() {
        // a tool call's arguments are a JSON string: the keys must be seen, not just the text
        let call = json!({"function": {"name": "x", "arguments": "{\"password\":\"pw-valor-um\",\"api_key\":\"chave-dois\",\"ok\":\"visivel\"}"}});
        let mut st = Stats::default();
        let out = Redactor::default().redact_value(&call, &mut st).to_string();
        assert!(!out.contains("pw-valor-um") && !out.contains("chave-dois"), "{out}");
        assert!(out.contains("visivel"), "what is not secret stays: {out}");
        assert_eq!(st.redacted, 2);
        // nested: JSON in a string in a string
        let nested = json!({"body": json!({"inner": "{\"token\": \"tk-valor-tres\"}"}).to_string()});
        assert!(!Redactor::default().redact_value(&nested, &mut Stats::default()).to_string().contains("tk-valor-tres"));
        // text that is JSON but holds no secret is returned as it was written (not re-serialised)
        let plain = "{\"b\": 1,   \"a\": [1, 2]}";
        assert_eq!(red(plain), plain);
        // prose that starts with a bracket is prose
        assert_eq!(red("[Gol] aos 12 minutos, token de acesso liberado"), "[Gol] aos 12 minutos, token de acesso liberado");
    }

    #[test]
    fn escaped_json_text_is_redacted_too() {
        // what a stringified message looks like: the quotes of the keys are escaped
        let text = r#"{\"content\":\"\",\"tool_calls\":[{\"function\":{\"arguments\":\"{\\\"password\\\":\\\"pw-valor-cinco\\\"}\"}}]} e \"api_key\":\"chave-seis\" ou password=\"pw-sete\""#;
        let out = red(text);
        for secret in ["pw-valor-cinco", "chave-seis", "pw-sete"] {
            assert!(!out.contains(secret), "{secret} in {out}");
        }
    }

    #[test]
    fn arguments_that_do_not_parse_are_omitted_when_they_look_secret() {
        let r = Redactor::default();
        let bad = json!({"function": {"arguments": "{\"password\": \"pw-valor-quatro\", oops"}});
        let mut st = Stats::default();
        let out = r.redact_value(&bad, &mut st).to_string();
        assert!(!out.contains("pw-valor-quatro") && out.contains("[omitted: unparsed arguments]"), "{out}");
        // unparsed but ordinary: kept (the detectors still run)
        let fine = json!({"function": {"arguments": "{\"caminho\": \"~/Documentos\", "}});
        assert!(r.redact_value(&fine, &mut Stats::default()).to_string().contains("Documentos"));
        // unparsed with a known token prefix and no key name: omitted too
        let tok = json!({"arguments": format!("{{\"x\": \"{}\", ", concat!("ghp", "_abcdefghijklmnopqrstuvwxyz0123456789"))});
        assert!(r.redact_value(&tok, &mut Stats::default()).to_string().contains("[omitted: unparsed arguments]"));
    }

    #[test]
    fn what_the_structured_pass_leaves_still_goes_through_the_text_detectors() {
        // the structured pass flags the password; the key that is a token and the PEM in a value are for the detectors
        let token = concat!("ghp", "_abcdefghijklmnopqrstuvwxyz0123456789");
        let block = test_pem("RSA", "MIIEowIBAAKCAQEA", true).replace('\n', "\\n");
        let text = format!("{{\"password\":\"x\",\"{token}\":\"y\",\"nota\":\"{block}\"}}");
        let out = red(&text);
        assert!(!out.contains(token), "a key that is a token: {out}");
        assert!(!out.contains("MIIEow"), "{out}");
        assert!(!out.contains("\"x\""), "{out}");
        // and the keys of an object value too
        let v = Redactor::default().redact_value(&json!({ token: 1 }), &mut Stats::default()).to_string();
        assert!(!v.contains(token), "{v}");
    }

    #[test]
    fn an_escaped_value_with_backslashes_in_it_is_cut_at_its_own_closing_quote() {
        // level 1: the text is JSON written inside a string; the value has an escaped newline and an escaped quote
        let l1 = r#"{\"password\":\"abc\\ndef-cauda-um \\\" mais-cauda\",\"ok\":\"visivel\"}"#;
        let out = red(l1);
        for tail in ["abc", "cauda-um", "mais-cauda"] {
            assert!(!out.contains(tail), "{tail} in {out}");
        }
        assert!(out.contains("visivel"), "what follows the closing quote stays: {out}");
        // level 0, with an escaped quote inside a quoted value
        let l0 = r#"password="abc\"def-cauda-zero" outro=1"#;
        let out0 = red(l0);
        assert!(!out0.contains("cauda-zero") && out0.contains("outro=1"), "{out0}");
        // level 2
        let l2 = r#"{\\\"api_key\\\":\\\"chave\\\\ncauda-dois\\\",\\\"ok\\\":1}"#;
        let out2 = red(l2);
        assert!(!out2.contains("cauda-dois") && !out2.contains("chave"), "{out2}");
    }

    /// The gaps the blind critic found (E2E-4, round 3): each value must go, in the redacted text and for the final
    /// scan, and the ordinary text next to them stays.
    #[test]
    fn the_critics_leaks_are_closed() {
        let aws = concat!("wJalrXUtnFEMI", "/K7MDENG/bPxRfi", "CYzx9Q2KEYk3Lp7");
        let short = concat!("aB3dE5fG", "7hJ9kL1mN");
        let split = format!("{} {}", concat!("ghp", "_ABCDEFGHIJ"), "KLMNOPQRSTUVWXYZ0123456789");
        let cases: Vec<(String, &str)> = vec![
            ("a senha é hunter2 e o resto".into(), "hunter2"),
            ("Minha senha do wifi é: Kalango-2024!".into(), "Kalango-2024"),
            ("my password is s3cr3tP4ss.".into(), "s3cr3tP4ss"),
            ("The password was correct-horse-9 yesterday".into(), "correct-horse-9"),
            ("GET /login?user=ana&password%3Dhunter3%26next%3D1".into(), "hunter3"),
            ("next=%2Fx%26passwd%3dvalor-escondido".into(), "valor-escondido"),
            (r#"{"pwd": "pw-um", "pass": "pw-dois", "auth": "pw-tres", "pin": "4321", "db_pass": "pw-quatro"}"#.into(), "pw-"),
            (r#"{"user_pin": 98765}"#.into(), "98765"),
            ("password=123456 depois".into(), "123456"),
            ("mysql -u root -pS3nhaForte banco".into(), "S3nhaForte"),
            ("mysqldump -uroot -phunter4 app > a.sql".into(), "hunter4"),
            (format!("aws_secret {aws} fim"), aws),
            (format!("chave {short} fim"), short),
            (format!("token {split} fim"), "KLMNOPQRSTUVWXYZ0123456789"),
            ("curl -H 'X-Auth: abc123xyz' https://x".into(), "abc123xyz"),
        ];
        let r = Redactor::default();
        for (text, secret) in &cases {
            let out = red(text);
            assert!(!out.contains(secret), "{secret} leaked from {text:?}: {out}");
            assert!(r.scan_text(text) > 0, "the final scan must see the secret in {text:?}");
        }
        let split_head = concat!("ghp", "_ABCDEFGHIJ");
        assert!(!red(&format!("token {split} fim")).contains(split_head), "both parts of a split token go");
        // the JSON keys: what is not a secret key stays (a substring is not enough)
        let v = r.redact_value(
            &json!({"pwd": "a", "pass": "b", "auth": "c", "pin": 1234, "x_auth": "d", "author": "Ana", "mapping": "m", "passageiro": "Rui", "shipping": "s", "spin": 3, "max_tokens": 512}),
            &mut Stats::default(),
        );
        for k in ["pwd", "pass", "auth", "pin", "x_auth"] {
            assert_eq!(v[k], REDACTED, "{k}");
        }
        assert_eq!(
            (v["author"].as_str(), v["mapping"].as_str(), v["passageiro"].as_str(), v["shipping"].as_str()),
            (Some("Ana"), Some("m"), Some("Rui"), Some("s"))
        );
        assert_eq!(v["max_tokens"], 512, "a count under a token-like key stays");
        assert_eq!(r.scan_value(&json!({"pin": 1234})), 1, "a PIN left as a number is a leak for the final scan");
    }

    /// What must not be touched by the new detectors (false positives).
    #[test]
    fn the_new_detectors_leave_ordinary_text_alone() {
        for ok in [
            "the token is rotated daily",
            "a senha é obrigatória para entrar",
            "the password is wrong, try again",
            "a senha é forte",
            "mkdir -p /tmp/x && grep -p foo",
            "tokens: 120 used, pin_required: true",
            "/home/ana/Documentos/Relatorio2024/final",
            "HttpClient2Builder relatorio2024final kalango_agente_motor_turn_state",
            "o passageiro Rui comprou o mapping de rotas, author Ana",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "123e4567-e89b-12d3-a456-426614174000",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// Text from outside with multi-byte characters where the detectors look byte by byte (after `%`, between the
    /// words of a command, next to a key): no panic, the secret still goes, and nothing else is cut.
    #[test]
    fn multi_byte_text_never_panics_and_the_secrets_still_go() {
        let r = Redactor::default();
        // after '%': one byte then a multi-byte character, or one right away
        for t in ["100%€", "%a\u{e9}x", "%😀", "a%3\u{e9}", "password%3Dabc%€ fim", "password%3Dabc%a\u{e9} fim", "%\u{a0}%\u{3000}%\u{2003}"] {
            let out = red(t);
            assert!(!out.contains("abc"), "{t:?} -> {out:?}");
            let _ = r.scan_text(t);
        }
        // Unicode spaces (no-break, ideographic, em) and runs of separators between the words of a command line
        for sep in ["\u{a0}", "\u{3000}", "\u{2003}", "  ", " \u{3000} "] {
            let line = format!("mysql{sep}-u{sep}root{sep}-pSegredoForte1{sep}banco é ótimo 😀");
            let out = red(&line);
            assert!(!out.contains("SegredoForte1"), "{line:?} -> {out:?}");
            assert_eq!(out, format!("mysql{sep}-u{sep}root{sep}-p{REDACTED}{sep}banco é ótimo 😀"), "exactly the password goes");
            let ssh = format!("sshpass{sep}-p{sep}Outra-Senha-2{sep}ssh é");
            let out = red(&ssh);
            assert_eq!(out, format!("sshpass{sep}-p{sep}{REDACTED}{sep}ssh é"), "{ssh:?}");
        }
        // next to keys and in phrases
        for t in ["é password=hunter9 😀", "\u{3000}senha\u{3000}é\u{3000}hunter9\u{3000}", "my password is\u{2003}hunter9 é"] {
            let out = red(t);
            assert!(!out.contains("hunter9"), "{t:?} -> {out:?}");
        }
    }

    /// A range that does not fall on a character's edge is widened to whole characters, never sliced mid-character.
    #[test]
    fn replace_ranges_widens_to_whole_characters() {
        let s = "aé😀b";
        let mut st = Stats::default();
        // 'é' is bytes 1..3, the emoji 3..7: ranges that start or end inside them
        assert_eq!(replace_ranges(s, &[(2, 5)], "#", &mut st), "a#b");
        assert_eq!(replace_ranges(s, &[(0, 2)], "#", &mut st), "#😀b");
        assert_eq!(replace_ranges(s, &[(6, 99)], "#", &mut st), "aé#");
    }

    #[test]
    fn stats_count_what_was_redacted() {
        let mut st = Stats::default();
        Redactor::default().redact_value(&json!({"password": "x", "t": concat!("ghp", "_abcdefghijklmnopqrstuvwxyz0123456789")}), &mut st);
        assert_eq!(st.redacted, 2);
    }

    /// A low-entropy password (the shape the token detector cannot see), put together at run time.
    fn weak_pw() -> String {
        concat!("bana", "na2024").to_string()
    }

    /// The secret leaves the text and the final scan sees it.
    #[track_caller]
    fn assert_gone(text: &str, secret: &str) {
        let out = red(text);
        assert!(!out.contains(secret), "{secret} leaked from {text:?}: {out:?}");
        assert!(Redactor::default().scan_text(text) > 0, "the final scan must see the secret in {text:?}");
    }

    /// Labels in the other languages of the interface (es, fr, de, it, ja, ru, zh), in pairs and in sentences.
    #[test]
    fn labels_in_the_other_languages_go() {
        let pw = weak_pw();
        for t in [
            format!("mi contraseña es {pw} gracias"),
            format!("contraseña: {pw}"),
            format!("mot de passe: {pw}"),
            format!("mon mot de passe est {pw}"),
            format!("Passwort: {pw}"),
            format!("Kennwort: {pw}"),
            format!("mein Passwort ist {pw}"),
            format!("la mia password è {pw}"),
            format!("parola d'ordine: {pw}"),
            format!("パスワード: {pw}"),
            format!("パスワードは{pw}です"),
            format!("пароль: {pw}"),
            format!("мой пароль {pw}"),
            format!("密码：{pw}"),
            format!("密码是{pw}"),
            format!("口令：{pw}"),
        ] {
            assert_gone(&t, &pw);
        }
        // the value is cut, not the quotes around it
        assert_eq!(red("{\"password\":\"日本語パスワード\"} ok"), format!("{{\"password\":\"{REDACTED}\"}} ok"));
        // the same words in ordinary sentences stay
        for ok in [
            "La contraseña debe tener al menos ocho caracteres.",
            "Das Passwort ist zu kurz.",
            "密码学是研究加密的科学。",
            "请输入您的密码。",
            "Пароль должен содержать не менее 8 символов.",
        ] {
            assert_eq!(red(ok), ok);
        }
    }

    /// JSON keys in the other languages.
    #[test]
    fn json_keys_in_the_other_languages_go() {
        let pw = weak_pw();
        let r = Redactor::default();
        for key in ["contraseña", "mot de passe", "Passwort", "Kennwort", "parola d'ordine", "パスワード", "пароль", "密码", "口令"] {
            let v = r.redact_value(&json!({ key: pw }), &mut Stats::default());
            assert_eq!(v[key], REDACTED, "{key}");
            assert_eq!(r.scan_value(&json!({ key: pw })), 1, "{key}");
            assert_gone(&json!({ key: pw }).to_string(), &pw);
            assert_gone(&json!({ "nota": json!({ key: pw }).to_string() }).to_string(), &pw);
        }
    }

    /// A label of several words, and the names that are a secret only when the value looks like a credential.
    #[test]
    fn longer_labels_and_weak_names_go() {
        let pw = weak_pw();
        for t in [
            format!("minha senha de acesso: {pw}"),
            format!("senha do wifi: {pw}"),
            format!("the password for the server: {pw}"),
            format!("o token é {pw}"),
            format!("the pw is {pw}"),
            format!("api key is {pw}"),
            format!("my api key: {pw}"),
            format!("la clave es {pw}"),
            format!("**Password:** {pw}"),
        ] {
            assert_gone(&t, &pw);
        }
        for ok in [
            "the token is rotated daily",
            "The API key is passed in the Authorization header.",
            "The pw is in the vault.",
            "a chave é girar à direita",
        ] {
            assert_eq!(red(ok), ok);
        }
    }

    /// A label alone on its line, the password alone on the next one.
    #[test]
    fn a_label_alone_on_its_line_takes_the_next_line() {
        let pw = weak_pw();
        for t in [format!("senha\n{pw}"), format!("Senha:\n{pw}\nfim"), format!("**Password**\n  {pw}\n"), format!("mot de passe\n{pw}")] {
            assert_gone(&t, &pw);
        }
        assert_eq!(red(&format!("Senha:\n{pw}\nfim")), format!("Senha:\n{REDACTED}\nfim"));
        for ok in ["Senha\nEsqueci minha senha", "Username\nPassword\nLogin", "Senha:"] {
            assert_eq!(red(ok), ok);
        }
    }

    /// A passphrase of several words goes whole, to the end of its clause; the rest of the text stays.
    #[test]
    fn passphrases_of_several_words_go_whole() {
        let words = concat!("correct horse ", "battery staple");
        let cake = concat!("bolo de ", "cenoura");
        for (t, secret) in [
            (format!("my password is {words}"), "horse"),
            (format!("my password is {words}. Next line"), "staple"),
            (format!("senha é {cake}"), "cenoura"),
            (format!("minha senha é {cake} 2024"), "cenoura"),
            (format!("senha: {cake}"), "cenoura"),
            (format!("passphrase: {words}"), "battery"),
            (format!("my password is \"{words}\" ok"), "horse"),
        ] {
            assert_gone(&t, secret);
        }
        assert_eq!(red(&format!("my password is {words}. Next line")), format!("my password is {REDACTED}. Next line"));
        assert_eq!(red(&format!("senha: {cake}, usuário: ana")), format!("senha: {REDACTED}, usuário: ana"));
        // sentences about a password are not passphrases
        for ok in [
            "the password is wrong, try again",
            "a senha é obrigatória para entrar",
            "Remember: the password is case sensitive.",
            "The password was changed yesterday.",
            "The password is hashed with Argon2 before it is stored.",
            "A senha nunca é enviada em texto puro.",
            "- `password`: the user's password (required)",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// Flags named after a secret take the next word as their value.
    #[test]
    fn command_line_flags_go() {
        let pw = weak_pw();
        for t in [
            format!("tool --password {pw} --x"),
            format!("gh auth login --with-token --token {pw}"),
            format!("app --db-password {pw}"),
            format!("app --api-key {pw}"),
        ] {
            assert_gone(&t, &pw);
        }
        for ok in [
            "docker login -u user --password-stdin < token.txt",
            "gh auth login --with-token < token.txt",
            "app --token-file ~/.config/t --x",
            "app --no-password run",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// The commands that take a password as a plain argument.
    #[test]
    fn passwords_given_to_known_commands_go() {
        let pw = weak_pw();
        // (the command name put together at run time: a secret scanner reads `curl -u user:x` as a credential)
        let client = concat!("cu", "rl");
        for t in [
            format!("mysql -p {pw}"),
            format!("mysql -u root -p {pw} db"),
            format!("htpasswd -b file user {pw}"),
            format!("htpasswd -nb user {pw}"),
            format!("{client} -u admin:{pw} https://x"),
            format!("{client} --user admin:{pw} https://x"),
            format!("echo {pw} | sudo -S apt update"),
            format!("sudo -S <<< {pw}"),
        ] {
            assert_gone(&t, &pw);
        }
        assert_eq!(red(&format!("htpasswd -b file user {pw}")), format!("htpasswd -b file user {REDACTED}"));
        for ok in [
            "mysql -u root -p mydb < dump.sql",
            "mysql -u root -p",
            "htpasswd -c /etc/nginx/.htpasswd ana",
            "curl -u admin https://x",
            "echo hello | tr a-z A-Z",
            "sudo -S apt update",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// `<password>X</password>` and separators written as HTML entities.
    #[test]
    fn markup_and_html_entities_go() {
        let pw = weak_pw();
        for t in [
            format!("<password>{pw}</password>"),
            format!("<senha> {pw} </senha>"),
            format!("password&#61;{pw}"),
            format!("password&#x3D;{pw}&amp;x=1"),
            format!("password&equals;{pw}"),
        ] {
            assert_gone(&t, &pw);
        }
        assert_eq!(red(&format!("<password>{pw}</password>")), format!("<password>{REDACTED}</password>"));
        for ok in ["<label for=\"password\">Senha</label>", "<password-field></password-field>", "&lt;token&gt; is a placeholder"] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// Names disguised with zero-width characters, Cyrillic or Greek look-alikes or full-width letters are still names;
    /// what is cut is the value as it was written.
    #[test]
    fn disguised_names_are_still_seen() {
        let pw = weak_pw();
        for t in [
            format!("pass\u{200b}word: {pw}"),
            format!("pass\u{feff}word={pw}"),
            format!("p\u{430}ssword: {pw}"),
            format!("t\u{3bf}ken: {pw}"),
            format!("\u{455}\u{435}nh\u{430}: {pw}"),
            format!("ｐａｓｓｗｏｒｄ：{pw}"),
            format!("my p\u{430}ssword is {pw}"),
        ] {
            assert_gone(&t, &pw);
        }
        assert_eq!(red(&format!("ｐａｓｓｗｏｒｄ：{pw} 終わり")), format!("ｐａｓｓｗｏｒｄ：{REDACTED} 終わり"));
        assert_eq!(red(&format!("p\u{430}ss\u{200b}word: {pw} ok")), format!("p\u{430}ss\u{200b}word: {REDACTED} ok"));
        let v = Redactor::default().redact_value(&json!({ "pass\u{200b}word": pw, "p\u{430}ssword": pw }), &mut Stats::default());
        assert!(!v.to_string().contains(&pw), "{v}");
        // Russian words with letters that look Latin stay as they are
        assert_eq!(red("Ключ лежит на столе."), "Ключ лежит на столе.");
    }

    /// One-time codes (2FA, OTP, verification codes).
    #[test]
    fn one_time_codes_go() {
        let code = concat!("482", "913");
        for t in [
            format!("meu código 2FA é {code}"),
            format!("my OTP is {code}"),
            format!("o código de verificação é {code}"),
            format!("your verification code is {code}"),
            format!("verification code: {code}"),
            format!("the 2FA code is {code}"),
        ] {
            assert_gone(&t, code);
        }
        for ok in ["The verification code is sent by email.", "Your OTP is valid for five minutes.", "The error code is 404."] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// The final scan reads the keys of JSON written as text, as the redaction does: a key that only JSON decoding
    /// shows (`pass\u0077ord`), a list under a secret key.
    #[test]
    fn the_final_scan_reads_the_keys_of_json_text() {
        let r = Redactor::default();
        let pw = weak_pw();
        for text in [format!("{{\"pass\\u0077ord\": \"{pw}\"}}"), json!({ "tokens": [pw, "outro"] }).to_string()] {
            assert!(!red(&text).contains(&pw), "{text}");
            assert!(r.scan_text(&text) > 0, "the final scan must see the secret in {text}");
            assert_eq!(r.scan_text(&red(&text)), 0, "what was redacted is clean: {}", red(&text));
        }
    }

    /// Code and configuration that name a secret without holding one (the critic's false positives).
    #[test]
    fn code_that_names_a_secret_is_left_alone() {
        for ok in [
            // calls
            "token = lexer.next_token()",
            "let secret_len = secret.len();",
            "const token = await getToken();",
            // types
            "password: str = Field(...)",
            "token: String,",
            // booleans
            "password_reset_email_sent = True",
            "tokenizer.encode(text, add_special_tokens=False)",
            // references to variables and the environment
            "API_KEY = os.environ[\"API_KEY\"]",
            "const apiKey = process.env.API_KEY;",
            "export GITHUB_TOKEN=${GITHUB_TOKEN:-}",
            "export API_KEY=$API_KEY",
            "let password = std::env::var(\"DB_PASSWORD\")?;",
            "self.secret = secret",
            // a list of grammar words in prose
            "O lexer gera tokens: IDENT, NUMBER, STRING.",
            // keys about a secret, not the secret
            "password_min_length = 12",
            "token_endpoint: https://auth.example.com/oauth/token",
            "secret_rotation_days = 90",
            // a prompt, a block, a log line
            "password = getpass.getpass('Password: ')",
            "read -s -p 'Password: ' password",
            "if not token: raise AuthError('missing token')",
            "export const SECRET = process.env.JWT_SECRET!;",
            "2026-11-26T10:12:03Z INFO auth: token refreshed for user=ana",
            "auth=ok latency=12ms",
            "secret: !vault |",
            "if password != confirm: raise ValueError('passwords differ')",
            "session.headers.update({'Authorization': f'Bearer {token}'})",
            "curl -H \"Authorization: Bearer $TOKEN\" https://api.example.com",
            "const token = req.headers.authorization?.split(' ')[1];",
            "const { password, ...rest } = user;",
            "printf 'password: %s\\n' \"$masked\"",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
        // and the same names with literals still go
        let pw = weak_pw();
        for t in [format!("token = {pw}"), format!("password: {pw}"), format!("API_KEY = \"{pw}\""), format!("export API_KEY=$ecret{pw}")] {
            assert_gone(&t, &pw);
        }
    }
}
