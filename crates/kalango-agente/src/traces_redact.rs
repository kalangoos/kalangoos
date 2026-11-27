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
    // other spellings and other secrets
    "psswd",
    "pswd",
    "pword",
    "passcode",
    "passkey",
    "creds",
    "signing_key",
    "encryption_key",
    "master_key",
    "auth_header",
    "mnemonic",
    "seed_phrase",
];

/// Keys that hold a secret only when their value is text (`{"session": "…"}`; `{"session": {"id": 7}}` is an object
/// about it, `{"seed": 42}` a number), compared as the whole key or its last word.
const TEXT_SECRET_KEYS: &[&str] = &["session", "sessionid", "otp", "seed", "dsn", "bearer"];

/// Short key names that are secrets only as a whole word at the end of the key (`pwd`, `db_pass`, `X-Auth`, `userPin`),
/// never as a piece of another word (`author`, `passageiro`, `mapping`, `spin`).
pub const SUFFIX_KEYS: &[&str] = &["pwd", "pass", "auth", "pin", "pw", "cred", "mdp", "psk"];

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
/// `--password-file`, `secret_rotation_days`). Only the text heuristics use them; a JSON key keeps a string value only
/// when it is not denied at all. (`hint`, `prompt`, `field`, `label`, `env`, `var`, `stdin` and `required` are left out:
/// a value under them can be the secret itself.)
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
    "mode",
    "scope",
    "scopes",
    "algorithm",
    "kind",
    "enabled",
    "rotation",
    "domain",
    "provider",
    "version",
    "regex",
    "pattern",
    "backend",
    "ref",
    "store",
    "attempts",
    "lifetime",
    "namespace",
    "retries",
    "tries",
    "failures",
    "at",
    "date",
    "time",
    "timestamp",
    "changed",
    "updated",
    "created",
    "since",
    "title",
    "placeholder",
    "description",
    "tooltip",
    "caption",
    "message",
];

/// Keys about tokens of text, not credentials (`"tokenizer": "lfm3"`).
const NOT_SECRET_KEYS: &[&str] = &["tokenizer", "tokenizers", "tokenization", "tokenize", "detokenize"];

/// A key of two or more words whose last word only describes the secret (see [`ABOUT_A_SECRET`]).
fn describes_a_secret(key: &str) -> bool {
    let words = key_words(key);
    words.len() >= 2
        && (words.last().is_some_and(|w| ABOUT_A_SECRET.contains(&w.as_str()))
            || words.first().is_some_and(|w| matches!(w.as_str(), "label" | "placeholder" | "title" | "tooltip" | "caption")))
}

impl Redactor {
    /// A redactor with its own deny list (the tests build ones with a key left out to show each key is needed).
    pub fn new(deny: &[&str]) -> Redactor {
        Redactor { deny: deny.iter().map(|k| normalize(k)).collect() }
    }

    /// A key named after a secret, as a JSON key is read: fail closed, whatever its last word says.
    fn denied_key(&self, key: &str) -> bool {
        let k = normalize(key);
        !k.is_empty()
            && !NOT_SECRET_KEYS.contains(&k.as_str())
            && (self.deny.iter().any(|d| k.contains(d.as_str())) || suffix_key(key).is_some() || leet_password(&k))
    }

    /// Whether a JSON key loses this value: a denied key (or one of [`TEXT_SECRET_KEYS`] with text), unless the value is
    /// a count, a boolean, null, a mask (`"***"`, `"..."`), or text that only describes the secret under a key that
    /// describes it (`"token_type": "bearer"`; `"password_policy": "banana2024"` still goes).
    fn json_secret(&self, key: &str, v: &Value) -> bool {
        let text = v.as_str();
        let named = self.denied_key(key) || (text.is_some() && text_secret_key(key));
        named
            && !self.keeps_number(key, v)
            && !matches!(v, Value::Bool(_) | Value::Null)
            && !text.is_some_and(|t| mask(t) || json_reference(t.trim(), key) || (describes_a_secret(key) && describing_value(key, t)))
    }

    /// A name in text that points at a secret: a denied key that does not only describe the secret (`password_policy:
    /// strict`, `token_endpoint: https://…`).
    fn denied(&self, key: &str) -> bool {
        self.denied_key(key) && !describes_a_secret(key)
    }

    /// A number kept under a denied JSON key: a count (`max_tokens`), or a number about a secret (`password_min_length`).
    fn keeps_number(&self, key: &str, v: &Value) -> bool {
        matches!(v, Value::Number(_)) && (!self.numbers_are_secret(key) || describes_a_secret(key))
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
                        if ENV_KEYS.contains(&kn.as_str()) && !x.is_null() && !only_references(x) {
                            stats.redacted += 1;
                            return (k.clone(), Value::String("[REDACTED:env]".into()));
                        }
                        if self.json_secret(k, x) {
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
                    let leaked_env = ENV_KEYS.contains(&kn.as_str()) && !x.is_null() && !only_references(x);
                    let leaked_secret = self.json_secret(k, x);
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
        // (every `${{ … }}` is one unit: the detectors see it as a mask, `***`, and nothing inside it is cut; a secret
        // glued to it, `${{ x }}banana2024`, still is)
        let units = gha_expressions(&view.text);
        let masked = mask_spans(&view.text, &units);
        let t: &str = &masked;
        let mut ranges = self.key_value_ranges(t);
        ranges.extend(phrase_ranges(t));
        ranges.extend(next_line_ranges(t));
        ranges.extend(self.markup_ranges(t));
        ranges.extend(self.command_ranges(t));
        ranges.extend(cjk_ranges(t));
        ranges.extend(gha_rejected_ranges(t));
        ranges.extend(scheme_ranges(t));
        ranges.extend(self.name_value_ranges(t));
        ranges.extend(self.code_literal_ranges(t));
        let ranges = outside_units(t, ranges, &units);
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
        // (only the values that changed are written again, where they were: the layout and the order of the keys stay)
        let lead = s.len() - s.trim_start().len();
        Some(match patch_json(t, &out) {
            Some(p) => format!("{}{p}{}", &s[..lead], &s[lead + t.len()..]),
            None => out.to_string(),
        })
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
            let st = s[..e]
                .char_indices()
                .rev()
                .take_while(|(_, c)| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '@'))
                .last()
                .map_or(e, |(k, _)| k);
            // (a label of several words names a code or a password: its value goes even when it is a number)
            // (a name that only describes a secret, `password_policy`, a generic one, `key`, `chave`, or a parameter
            // of a URL, `?sig=`, holds a secret only when its value looks like one: `shaped_only`)
            let word = &s[st..e];
            let query = st > 0 && matches!(b[st - 1], b'?' | b'&') && QUERY_SECRET_NAMES.contains(&word.to_ascii_lowercase().as_str());
            let generic = GENERIC_SECRET_NAMES.contains(&fold_accents(word).as_str());
            let (strong, by_label, shaped_only) = if st < e && self.denied(word) {
                (password_label(word), false, false)
            } else if st < e && (query || generic || (self.denied_key(word) && describes_a_secret(word))) {
                (false, false, true)
            } else if let Some(strong) = label_before(s, e) {
                (strong, true, false)
            } else {
                i += 1;
                continue;
            };
            let name = &s[st..e];
            // ("Boarding pass: seat 14C" is a ticket)
            if name.eq_ignore_ascii_case("pass") && pass_ticket(s, st) {
                i += 1;
                continue;
            }
            // a cookie header: every pair loses its value (the attributes of `Set-Cookie` stay)
            if matches!(normalize(name).as_str(), "cookie" | "setcookie") {
                let end = s[i..].find('\n').map_or(s.len(), |n| i + n);
                ranges.extend(cookie_values(s, i + sep_len, end));
                i = end.max(i + 1);
                continue;
            }
            // the value after it (markdown emphasis closing the label, `**Password:** X`, is not the value)
            let mut vs = i + sep_len;
            while vs < b.len() && matches!(b[vs], b' ' | b'\t') {
                vs += 1;
            }
            // (a YAML block, `password: |`: the value is the next line)
            if colon {
                let ve0 = vs + b[vs..].iter().take_while(|c| matches!(c, b'|' | b'>' | b'-' | b'+')).count();
                if matches!(&s[vs..ve0], "|" | ">" | "|-" | ">-" | "|+" | ">+") && s[ve0..].trim_start_matches([' ', '\t']).starts_with('\n') {
                    let next = ve0 + s[ve0..].find('\n').unwrap_or(0) + 1;
                    vs = next + s[next.min(s.len())..].len() - s[next.min(s.len())..].trim_start_matches([' ', '\t']).len();
                }
            }
            // (a masked `${{ … }}` is the value: alone, nothing to cut; with a word glued to it, `${{ x }}banana2024`,
            // that word when it looks like a credential, `${{ x }}-latest` is not one)
            let stars = b[vs..].iter().take_while(|c| **c == b'*').count();
            let masked_unit = stars >= 3;
            if masked_unit && b.get(vs + stars).is_none_or(|c| c.is_ascii_whitespace()) {
                i += 1;
                continue;
            }
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
            // (under a password's name after `:`, `password: none`, `password: true`: the word is the password; a code's
            // `None`/`True` after `=` is a value of the language, and a JSON key's `true` a boolean)
            let json_key = s[e..i].contains('"');
            let word_as_password = strong
                && colon
                && !json_key
                && names_the_password(name)
                && matches!(s[vs..value_end(b, vs)].to_ascii_lowercase().as_str(), "true" | "false" | "none");
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
                    // (an expression in the quotes may hold the same quote: `'${{ x == 'y' }}'` runs to its `}}`)
                    if s[vs..].starts_with("${{") {
                        let line = window(s, vs, 512);
                        let line = &line[..line.find('\n').unwrap_or(line.len())];
                        if let Some(e) = line.find("}}").filter(|e| vs + e + 2 > ve) {
                            ve = vs + e + 2;
                        }
                    }
                    if ve < b.len() && b[ve] == q {
                        ve -= depth;
                    } else if s[vs..ve].starts_with("${{") && s[vs..ve].ends_with("}}") {
                        // (the expression ends the value)
                    } else if s[vs..ve].trim().is_empty() || s[vs..ve].starts_with([')', ' ', ',', ';', '\t']) {
                        // (a quote that never closes, followed by nothing like a value, ends the string that holds the
                        // name: `input('Password: ')`)
                        i = vs.max(i + 1);
                        continue;
                    }
                    // (an interpolation, a mask or a placeholder; under a name that is not a password's, text that only
                    // names the secret: `"jwt-secret-name"`, `"X-API-Key"`)
                    let quoted = &s[vs..ve];
                    if interpolation_only(quoted)
                        || mask(quoted)
                        || placeholder(quoted)
                        || (!strong && (names_a_secret(quoted) || quoted_prose(quoted)))
                    {
                        i = ve.max(i + 1);
                        continue;
                    }
                }
                None => {
                    // (a YAML tag before the value, `!!str X`, `!vault X`: the value is the word after it)
                    if b.get(vs) == Some(&b'!') {
                        let te = value_end(b, vs);
                        let mut w = te;
                        while w < b.len() && matches!(b[w], b' ' | b'\t') {
                            w += 1;
                        }
                        if w > te && w < b.len() && !matches!(b[w], b'\n' | b'\r' | b'|' | b'>') {
                            vs = w;
                        }
                    }
                    ve = value_end(b, vs);
                    // (a `$(…)`, `${…}` or `{{…}}` is judged, and cut, whole: `$(echo X)`)
                    for (open, close) in [("$(", ")"), ("${{", "}}"), ("${", "}"), ("{{", "}}")] {
                        if s[vs..].starts_with(open) {
                            // (on its line, at most 256 bytes ahead)
                            let window = &b[vs..(vs + 256).min(b.len())];
                            let window = &window[..window.iter().position(|c| *c == b'\n').unwrap_or(window.len())];
                            if let Some(e) = window.windows(close.len()).position(|w| w == close.as_bytes()) {
                                ve = ve.max(vs + e + close.len());
                            }
                        }
                    }
                    if !word_as_password && !self.value_shaped(&s[vs..ve], &s[ve..], colon, name) {
                        i = ve.max(i + 1);
                        continue;
                    }
                    // "Authorization: Bearer abc": the scheme is not the secret
                    let word = s[vs..ve].trim_end_matches(':').to_ascii_lowercase();
                    if word == "digest" {
                        // (the parameters of a digest go to the end of the line: `response="…"` is the secret)
                        ve = s[ve..].find('\n').map_or(s.len(), |n| ve + n);
                    } else if AUTH_SCHEMES.contains(&word.as_str()) {
                        let mut w = ve;
                        while w < b.len() && matches!(b[w], b' ' | b'\t') {
                            w += 1;
                        }
                        let mut we = w;
                        while we < b.len() && !b[we].is_ascii_whitespace() && !matches!(b[we], b'"' | b'\'' | b'`' | b',' | b';') {
                            we += 1;
                        }
                        // (a `$(…)` after the scheme is judged, and cut, whole: `Bearer $(echo X)`)
                        if s[w..].starts_with("$(") {
                            if let Some(e) = window(s, w, 256).split('\n').next().and_then(|l| l.find(')')) {
                                we = we.max(w + e + 1);
                            }
                        }
                        // (after a scheme the next word goes whatever its shape; "auth: token refreshed" is a log line,
                        // and `Bearer $TOKEN` names a variable)
                        let next = &s[w..we.max(w)];
                        let common = COMMON_AFTER_SCHEME.contains(&fold_accents(next.trim_end_matches(['.', ',', ';', ':', '!', '?'])).as_str());
                        let variable = interpolation_at(next).is_some_and(|n| n == next.len())
                            || (next.starts_with(['$', '{']) && interpolation_at(window(s, w, 256).split('\n').next().unwrap_or("")).is_some());
                        if we > w && !common && !variable && !placeholder(next) && !mask(next) {
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
            // (a label of several words with a preposition, "senha do usuário:", is a sentence as often as a label: its
            // value goes when it looks like a credential)
            if (masked_unit && !literal_secret(value))
                || (by_label && !strong && !credential_shaped(value))
                || (shaped_only
                    && !lifetime_or_namespace(name)
                    && (!(literal_secret(value) || hex_key(value)) || (time_key(name) && timestamp(value))))
                // (a namespace is checked for a literal and a key in hex inside `describing_value`; a lifetime must be a
                // duration, which `literal_secret` would cut: `1500ms`)
                || (shaped_only && lifetime_or_namespace(name) && describing_value(name, value))
            {
                i = ve.max(i + 1);
                continue;
            }
            // (a number is a count, `tokens: 120`, except under a password or PIN key)
            let numeric =
                !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit() || c == b'.') && !self.numbers_are_secret(name) && !strong && !by_label;

            if value.is_empty() || numeric || value.starts_with("[REDACTED") || (matches!(value, "null" | "true" | "false") && !word_as_password) {
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
        // (the name of a secret as the value, `passwd: password updated`; a PowerShell cmdlet, `Read-Host`; a variable
        // of PowerShell's environment, `$env:CI_JOB_TOKEN`)
        let lower = value.to_ascii_lowercase();
        let verb_noun = value.split_once('-').is_some_and(|(v, n)| {
            v.len() > 2
                && n.len() > 2
                && v.starts_with(|c: char| c.is_ascii_uppercase())
                && n.starts_with(|c: char| c.is_ascii_uppercase())
                && value.bytes().all(|c| c.is_ascii_alphabetic() || c == b'-')
        });
        // (a known cmdlet; or another verb-noun given a parameter, `Get-Thing -Name x`, under a name that is not a
        // password's: `password: Hunter-Two` is a password)
        let parameter = rest.trim_start_matches([' ', '\t']).starts_with('-')
            && rest.trim_start_matches([' ', '\t']).as_bytes().get(1).is_some_and(u8::is_ascii_alphabetic);
        let cmdlet = verb_noun && (KNOWN_CMDLETS.contains(&lower.as_str()) || (parameter && !password_label(name)));
        // (another word after it on its line: only the bytes up to that word are read)
        let after = rest.trim_start_matches([' ', '\t']);
        let sentence = after.len() < rest.len() && !after.is_empty() && !after.starts_with(['\n', '\r']);
        if (matches!(lower.as_str(), "password" | "passwd" | "senha") && sentence)
            || cmdlet
            || (lower.starts_with("$env:") && variable_name(&value[5..]))
        {
            return false;
        }
        if !value_is_literal(value, rest) {
            return false;
        }
        // a YAML tag (`secret: !vault |`); a word under a bare `auth` that is not a credential (`auth=ok`)
        if (value.len() > 1 && value.starts_with('!') && value[1..].bytes().all(|c| c.is_ascii_alphabetic()))
            || (suffix_key(name) == Some("auth")
                && !credential_shaped(value)
                && !matches!(value.to_ascii_lowercase().as_str(), "bearer" | "basic" | "token" | "digest"))
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

    /// `<password>X</password>`, `<senha>X</senha>`: the text of an element named after a secret; `<add key="password"
    /// value="X"/>`, `<input type="password" value="X">`, `<entry key="token">X</entry>`: the value of an element whose
    /// key, name, type or id names a secret.
    fn markup_ranges(&self, s: &str) -> Vec<(usize, usize)> {
        let mut ranges = Vec::new();
        if !s.contains('<') {
            return ranges;
        }
        let mut from = 0;
        while let Some(p) = s[from..].find('<').map(|p| p + from) {
            from = p + 1;
            // (a tag within 1 KiB, the text of an element within 4 KiB: no search runs to the end of the text at every `<`)
            let Some(close) = window(s, p, 1024).find('>').map(|e| p + e) else { continue };
            let tag = &s[p..close];
            if !["key", "name", "type", "id"].iter().any(|a| attr(tag, a).is_some_and(|(x, y)| self.denied(&tag[x..y]))) {
                continue;
            }
            if let Some((x, y)) = attr(tag, "value") {
                let v = &tag[x..y];
                if !v.is_empty() && !v.starts_with("[REDACTED") && !mask(v) && !placeholder(v) {
                    ranges.push((p + x, p + y));
                }
            } else if !tag.ends_with('/') {
                // (the text of the element, up to its closing tag)
                if let Some(end) = window(s, close + 1, 4096).find("</").map(|e| close + 1 + e) {
                    let content = &s[close + 1..end];
                    let t = content.trim();
                    if !t.is_empty() && !t.contains('<') && !t.starts_with("[REDACTED") {
                        let at = close + 1 + (content.len() - content.trim_start().len());
                        ranges.push((at, at + t.len()));
                    }
                }
            }
        }
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
            let Some(open_end) = window(s, name_end, 1024).find('>').map(|e| name_end + e + 1) else { continue };
            if b[open_end - 2] == b'/' || !self.denied(name) {
                continue;
            }
            let Some(close) = window(s, open_end, 4096).find("</").map(|e| open_end + e) else { continue };
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
                        if sshpass || more || literal_secret(v) {
                            push(vat, v.len());
                        }
                    }
                } else if let Some(flag) = w
                    .strip_prefix("--")
                    .map(|f| f.trim_end_matches(['`', '"', '\'', ')', ',', '.']))
                    .filter(|f| !f.contains('=') && !f.starts_with("no-") && !f.starts_with("with-"))
                {
                    if curl && flag == "user" {
                        if let Some((vat, v)) = value_at(n + 1) {
                            if let Some(c) = v.find(':') {
                                push(vat + c + 1, v.len() - c - 1);
                            }
                        }
                    } else if self.denied(flag) {
                        // (`--password-stdin` reads it from the input: the word after it is the next argument, unless it
                        // looks like a credential; "the --token flag" is prose)
                        let stdin = flag.ends_with("-stdin");
                        if let Some((vat, v)) = value_at(n + 1)
                            .filter(|(_, v)| !matches!(*v, "flag" | "option" | "argument" | "parameter" | "switch") && (!stdin || literal_secret(v)))
                        {
                            push(vat, v.len());
                        }
                    }
                } else if curl && (w == "-u" || (w.starts_with("-u") && w.contains(':'))) {
                    let (vat, v) = if w == "-u" { value_at(n + 1).unwrap_or((at, "")) } else { (at + 2, &w[2..]) };
                    // (`curl -u "$USER:$PASSWORD"` names variables)
                    if let Some(c) = v.find(':').filter(|c| !shell_variable(v[c + 1..].trim_end_matches(['"', '\'']))) {
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
            // other tools that take a password after a flag (glued to it for 7z and unrar)
            for &(tool, flag, glued_flag) in TOOL_PASSWORD_FLAGS {
                let Some(c) = position(tool) else { continue };
                // (`docker -p` is a port, except in `docker login`)
                if matches!(tool, "docker" | "podman" | "nerdctl") && !words[c + 1..].iter().any(|(_, w)| *w == "login") {
                    continue;
                }
                for (n, &(at, w)) in words.iter().enumerate().skip(c + 1) {
                    if glued_flag && w.len() > flag.len() && w.starts_with(flag) {
                        push(at + flag.len(), w.len() - flag.len());
                    } else if !glued_flag && w == flag {
                        if let Some((vat, v)) = value_at(n + 1) {
                            // (`-pass pass:X`; `env:VAR` and `file:path` say where it is)
                            if let Some(x) = v.strip_prefix("pass:") {
                                push(vat + 5, x.len());
                            } else if !["env:", "file:", "fd:", "stdin"].iter().any(|p| v.starts_with(p)) {
                                push(vat, v.len());
                            }
                        }
                    }
                }
            }
            // `wpa_passphrase SSID X`; `vault login X`
            if let Some(c) = position("wpa_passphrase") {
                if let Some(&(vat, v)) = words[c + 1..].iter().filter(|(_, w)| !w.starts_with('-')).nth(1) {
                    push(vat, v.len());
                }
            }
            if let Some(c) = position("vault") {
                if words.get(c + 1).is_some_and(|(_, w)| *w == "login") {
                    if let Some((vat, v)) = value_at(c + 2).filter(|(_, v)| !v.contains('=') && credential_shaped(v)) {
                        push(vat, v.len());
                    }
                }
            }
            // `sudo -S <<< X`
            if let Some(su) = position("sudo") {
                let stdin = words[su + 1..].iter().any(|(_, w)| *w == "--stdin" || (w.starts_with('-') && !w.starts_with("--") && w.contains('S')));
                if stdin {
                    if let Some(h) = words.iter().position(|(_, w)| *w == "<<<") {
                        if let Some(&(vat, v)) = words.get(h + 1) {
                            push(vat, v.len());
                        }
                    }
                }
            }
            // a password piped into what reads it: `echo X | sudo -S …`, `| passwd --stdin`, `| chpasswd` (`user:X`),
            // `| docker login --password-stdin`
            if let Some(pipe) = words.iter().position(|(_, w)| *w == "|") {
                let right = &words[pipe + 1..];
                let reader = right.first().map(|(_, w)| command(w)).unwrap_or_default();
                let has = |x: &str| right.iter().any(|(_, w)| *w == x);
                let sudo_stdin = reader == "sudo"
                    && right[1..].iter().any(|(_, w)| *w == "--stdin" || (w.starts_with('-') && !w.starts_with("--") && w.contains('S')));
                let chpasswd = reader == "chpasswd";
                if sudo_stdin || chpasswd || (reader == "passwd" && has("--stdin")) || has("--password-stdin") {
                    if let Some(e) = words[..pipe].iter().rposition(|(_, w)| matches!(command(w).as_str(), "echo" | "printf")) {
                        for &(vat, v) in &words[e + 1..pipe] {
                            if (v.starts_with('-') && v.len() <= 3) || v.contains('%') {
                                continue;
                            }
                            match v.find(':').filter(|_| chpasswd) {
                                Some(c) => push(vat + c + 1, v.len() - c - 1),
                                None => push(vat, v.len()),
                            }
                        }
                    }
                }
            }
            line_start += line.len();
        }
        ranges
    }

    /// `- name: DB_PASSWORD` with `value: X` on the next line (Kubernetes, compose), `ENV PASSWORD X` (a Dockerfile),
    /// `set aws_secret_access_key X` (a key named after a secret as a word of a command): the name on one side, the
    /// value on the other.
    fn name_value_ranges(&self, s: &str) -> Vec<(usize, usize)> {
        let mut ranges = Vec::new();
        let mut lines: Vec<(usize, &str)> = Vec::new();
        let mut at = 0;
        for l in s.split_inclusive('\n') {
            lines.push((at, l));
            at += l.len();
        }
        for (n, &(at, line)) in lines.iter().enumerate() {
            let body = line.trim_end_matches(['\n', '\r']);
            let t = body.trim_start();
            let t = t.strip_prefix("- ").unwrap_or(t).trim_start();
            if let Some(name) = t.strip_prefix("name:") {
                if self.denied(name.trim().trim_matches(['"', '\''])) {
                    if let Some(&(nat, next)) = lines.get(n + 1) {
                        let nb = next.trim_end_matches(['\n', '\r']);
                        let nt = nb.trim_start();
                        if let Some(v) = nt.strip_prefix("value:") {
                            let raw = v.trim();
                            let inner = raw.trim_matches(['"', '\'']);
                            let quoted = raw.len() > inner.len();
                            if !inner.is_empty()
                                && !inner.starts_with("[REDACTED")
                                && (quoted || value_is_literal(inner, ""))
                                && !interpolation_only(inner)
                            {
                                let vat = nat
                                    + (nb.len() - nt.len())
                                    + "value:".len()
                                    + (v.len() - v.trim_start().len())
                                    + (raw.len() - raw.trim_start_matches(['"', '\'']).len());
                                ranges.push((vat, vat + inner.len()));
                            }
                        }
                    }
                }
            }
            let words: Vec<(usize, &str)> =
                body.split(char::is_whitespace).filter(|w| !w.is_empty()).map(|w| (w.as_ptr() as usize - line.as_ptr() as usize, w)).collect();
            // `ENV NAME value` (with `=` it is a pair, read as one)
            if words.len() >= 3 && matches!(words[0].1, "ENV" | "ARG") && !words[1].1.contains('=') && self.denied(words[1].1) {
                let vat = words[2].0;
                let v = body[vat..].trim_end();
                if value_is_literal(v, "") && !interpolation_only(v) {
                    ranges.push((at + vat, at + vat + v.len()));
                }
            }
            for pair in words.windows(2) {
                let (w, (vat, v)) = (pair[0].1, pair[1]);
                if w.contains('_')
                    && !w.contains(['=', ':', '(', '"', '\'', ',', ';'])
                    && self.denied(w)
                    && literal_secret(v)
                    && value_is_literal(v, "")
                    && !v.contains(['=', '('])
                {
                    ranges.push((at + vat, at + vat + v.len()));
                }
            }
        }
        ranges
    }

    /// A literal inside code that holds a secret (`password = get_secret("X")`, `os.environ.get("X") or "X"`, `password ==
    /// "X"`, `setPassword("X")`, `login(user, "X")`, `key = [X]`): on a line of code that names a secret or calls a
    /// login, a quoted literal or a bare word in brackets that looks like a credential goes; names (`"SECRET_KEY"`,
    /// `"accessToken"`), messages, paths and numbers stay.
    fn code_literal_ranges(&self, s: &str) -> Vec<(usize, usize)> {
        const LOGIN_CALLS: &[&str] = &["login", "connect", "authenticate", "auth", "signin", "sign_in", "logon"];
        let mut ranges = Vec::new();
        let mut base = 0;
        for line in s.split_inclusive('\n') {
            let at = base;
            base += line.len();
            // (JSON is read by its keys, not as code: a line that starts like it is left to that)
            let lead = line.trim_start();
            if !line.contains(['(', '=', '[']) || lead.starts_with(['{', '[', '"']) {
                continue;
            }
            let b = line.as_bytes();
            // an identifier named after a secret (inside quotes too: `config['password']`), or a login call
            let mut names_secret = false;
            let mut quoted = None;
            let mut i = 0;
            while i < b.len() && !names_secret {
                if !(b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    match quoted {
                        None if matches!(b[i], b'"' | b'\'' | b'`') => quoted = Some(b[i]),
                        Some(q) if b[i] == q => quoted = None,
                        _ => {}
                    }
                    i += 1;
                    continue;
                }
                let st = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                let id = &line[st..i];
                let call = b.get(i) == Some(&b'(');
                // (a bare `key` only as code, not inside a literal: `"key-2024-11"`)
                // (a count of tokens of text, `max_tokens`, is not a secret's name)
                names_secret = (id.eq_ignore_ascii_case("key") && quoted.is_none())
                    || (self.denied(id) && !id.to_ascii_lowercase().ends_with("tokens") && !id.to_ascii_lowercase().contains("tokeniz"))
                    || (call && LOGIN_CALLS.contains(&id.to_ascii_lowercase().as_str()));
            }
            if !names_secret {
                continue;
            }
            let mut i = 0;
            while i < b.len() {
                match b[i] {
                    q @ (b'"' | b'\'' | b'`') => {
                        let st = i + 1;
                        let Some(e) = line[st..].find(q as char).map(|e| st + e) else { break };
                        if code_secret(&line[st..e]) {
                            ranges.push((at + st, at + e));
                        }
                        i = e + 1;
                    }
                    b'(' | b'[' | b',' | b'=' => {
                        // a bare word as an argument, an element or the other side of a comparison: `get_secret(X)`,
                        // `[X]`, `password == X`
                        if b[i] == b'=' && !(i > 0 && b[i - 1] == b'=') {
                            i += 1;
                            continue;
                        }
                        let mut st = i + 1;
                        while st < b.len() && b[st] == b' ' {
                            st += 1;
                        }
                        let mut e = st;
                        while e < b.len() && (b[e].is_ascii_alphanumeric() || matches!(b[e], b'_' | b'-')) {
                            e += 1;
                        }
                        let closes_there = if b[i] == b'=' {
                            b.get(e).is_none_or(|c| matches!(c, b')' | b':' | b' ' | b'\n' | b'\r' | b';'))
                        } else {
                            matches!(b.get(e), Some(b')' | b']' | b','))
                        };
                        if e > st && closes_there && code_secret(&line[st..e]) {
                            ranges.push((at + st, at + e));
                        }
                        i += 1;
                    }
                    _ => i += 1,
                }
            }
        }
        ranges
    }
}

/// Tools that take a password after a flag: (command, flag, whether the password is glued to the flag).
const TOOL_PASSWORD_FLAGS: &[(&str, &str, bool)] = &[
    ("redis-cli", "-a", false),
    ("zip", "-P", false),
    ("unzip", "-P", false),
    ("7z", "-p", true),
    ("7za", "-p", true),
    ("7zz", "-p", true),
    ("unrar", "-p", true),
    ("rar", "-p", true),
    ("mongo", "-p", false),
    ("mongosh", "-p", false),
    ("ssh-keygen", "-N", false),
    ("ssh-keygen", "-P", false),
    ("openssl", "-k", false),
    ("openssl", "-pass", false),
    ("openssl", "-passin", false),
    ("openssl", "-passout", false),
    ("az", "-p", false),
    ("docker", "-p", false),
    ("podman", "-p", false),
    ("nerdctl", "-p", false),
];

/// The span of a quoted attribute's value inside a tag (`name="…"`), relative to the tag.
fn attr(tag: &str, name: &str) -> Option<(usize, usize)> {
    let b = tag.as_bytes();
    let mut from = 0;
    while let Some(p) = tag[from..].find(name).map(|p| p + from) {
        from = p + name.len();
        let mut j = p + name.len();
        while j < b.len() && b[j] == b' ' {
            j += 1;
        }
        if p == 0 || !b[p - 1].is_ascii_whitespace() || b.get(j) != Some(&b'=') {
            continue;
        }
        j += 1;
        while j < b.len() && b[j] == b' ' {
            j += 1;
        }
        let q = *b.get(j)?;
        if !matches!(q, b'"' | b'\'') {
            continue;
        }
        let e = tag[j + 1..].find(q as char)? + j + 1;
        return Some((j + 1, e));
    }
    None
}

/// The values of the pairs of a cookie header between `from` and `end` (`theme=dark; session=X`), not the attributes of
/// `Set-Cookie` (`Path`, `Domain`, `Expires`…).
fn cookie_values(s: &str, from: usize, end: usize) -> Vec<(usize, usize)> {
    const ATTRIBUTES: &[&str] =
        &["path", "domain", "expires", "max-age", "samesite", "secure", "httponly", "priority", "partitioned", "version", "comment"];
    let mut out = Vec::new();
    let mut at = from;
    for part in s[from..end].split(';') {
        if let Some((name, value)) = part.split_once('=') {
            let n = name.trim().to_ascii_lowercase();
            let v = value.trim();
            if !ATTRIBUTES.contains(&n.as_str()) && !v.is_empty() && !v.starts_with("[REDACTED") {
                let vs = at + name.len() + 1 + (value.len() - value.trim_start().len());
                out.push((vs, vs + v.len()));
            }
        }
        at += part.len() + 1;
    }
    out
}

/// PowerShell cmdlets that read or build a secret instead of holding one (`$password = Read-Host -AsSecureString`).
const KNOWN_CMDLETS: &[&str] = &[
    "read-host",
    "get-content",
    "get-credential",
    "convertto-securestring",
    "convertfrom-securestring",
    "get-secret",
    "get-storedcredential",
    "get-azkeyvaultsecret",
    "get-item",
    "get-itemproperty",
    "get-childitem",
    "import-clixml",
    "get-variable",
    "invoke-restmethod",
    "invoke-webrequest",
    "new-object",
];

/// At most `n` bytes of `s` from `at` (cut on a character), so a search does not run to the end of the text.
fn window(s: &str, at: usize, n: usize) -> &str {
    let mut end = (at + n).min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[at.min(end)..end]
}

/// The contexts and functions of GitHub Actions expressions.
const GHA_CONTEXTS: &[&str] = &["github", "env", "vars", "secrets", "matrix", "steps", "needs", "inputs", "job", "jobs", "runner", "strategy"];
const GHA_FUNCTIONS: &[&str] =
    &["format", "contains", "startswith", "endswith", "join", "tojson", "fromjson", "hashfiles", "success", "always", "cancelled", "failure"];

/// Whether the content of `${{ … }}` is an expression: names of a context (`secrets.X`, `matrix.os`, `github.token`),
/// known functions (`format('{0}', secrets.X)`), operators, numbers, `true`/`false`/`null`, and strings that do not
/// look like a credential. A pair (`password=X`), a bare word or a credential-shaped string is not one.
fn gha_expression(c: &str) -> bool {
    gha_expression_after(c, false)
}

/// [`gha_expression`] for a block after a name (`strict`: a password's or a PIN's name): a quoted string is an operand
/// of a comparison (`github.event_name == 'push'`) or an argument of a function (`format('{0}', …)`), never the whole
/// expression nor a fallback (`secrets.X || 'x'`); after a password's name, not even an operand that looks like a
/// credential, a number or a phrase.
fn gha_expression_after(c: &str, strict: bool) -> bool {
    let b = c.as_bytes();
    let mut i = 0;
    let mut any = false;
    // (how many of the open brackets belong to a call of a known function)
    let mut calls: Vec<bool> = Vec::new();
    let mut call_next = false;
    while i < b.len() {
        let ch = b[i];
        if ch == b'(' {
            calls.push(call_next);
            call_next = false;
            i += 1;
        } else if ch == b')' {
            calls.pop();
            i += 1;
        } else if ch.is_ascii_whitespace() || matches!(ch, b',' | b'[' | b']' | b'*') {
            i += 1;
        } else if ch == b'\'' {
            let Some(e) = c[i + 1..].find('\'').map(|e| i + 1 + e) else { return false };
            let lit = &c[i + 1..e];
            let before = c[..i].trim_end();
            let after = c[e + 1..].trim_start();
            let compared = ["==", "!=", "<=", ">=", "<", ">"].iter().any(|op| before.ends_with(op) || after.starts_with(op));
            let argument = calls.last() == Some(&true) && (before.ends_with('(') || before.ends_with(','));
            if !(compared || argument) || gha_secret_literal(lit) {
                return false;
            }
            if strict && (credential_shaped(lit) || lit.trim().contains(' ') || (!lit.is_empty() && lit.bytes().all(|c| c.is_ascii_digit()))) {
                return false;
            }
            i = e + 1;
            any = true;
        } else if matches!(ch, b'=' | b'!' | b'<' | b'>' | b'&' | b'|') {
            // (`==`, `!=`, `<=`, `>=`, `&&`, `||`, `!`, `<`, `>`: an `=` alone makes a pair)
            let op = b.get(i..i + 2).unwrap_or(&b[i..]);
            if ch == b'=' && op != b"==" {
                return false;
            }
            i += if matches!(op, b"==" | b"!=" | b"<=" | b">=" | b"&&" | b"||") { 2 } else { 1 };
        } else if ch.is_ascii_digit() {
            let st = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            // (after a password's or a PIN's name, a number of four digits or more is the value itself)
            if strict && i - st >= 4 {
                return false;
            }
            // (a number glued to letters is a word, not a number)
            if b.get(i).is_some_and(|c| c.is_ascii_alphabetic()) {
                return false;
            }
            any = true;
        } else if ch.is_ascii_alphabetic() || ch == b'_' {
            let st = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'-' | b'.')) {
                i += 1;
            }
            let path = &c[st..i];
            let head = path.split('.').next().unwrap_or("").to_ascii_lowercase();
            let call = b[i..].iter().find(|c| !c.is_ascii_whitespace()) == Some(&b'(');
            call_next = call && GHA_FUNCTIONS.contains(&head.as_str());
            if !(GHA_CONTEXTS.contains(&head.as_str())
                || (call && GHA_FUNCTIONS.contains(&head.as_str()))
                || matches!(path, "true" | "false" | "null"))
            {
                return false;
            }
            any = true;
        } else {
            return false;
        }
    }
    any
}

/// The spans of the GitHub Actions expressions, `${{ … }}` up to its `}}` on the same line (within 512 bytes).
fn gha_expressions(s: &str) -> Vec<(usize, usize)> {
    gha_blocks(s).into_iter().filter(|b| b.2).map(|b| (b.0, b.1)).collect()
}

/// Every closed `${{ … }}` (up to its `}}` on the same line, within 512 bytes): where it is, and whether it is an
/// expression (in its context: strictly after a password's or a PIN's name).
fn gha_blocks(s: &str) -> Vec<(usize, usize, bool)> {
    let mut blocks = Vec::new();
    let mut from = 0;
    while let Some(p) = s[from..].find("${{").map(|p| p + from) {
        let w = window(s, p + 3, 512);
        let w = &w[..w.find('\n').unwrap_or(w.len())];
        match w.find("}}") {
            // (only an expression is a unit; one that holds a pair or a credential is text the detectors read)
            Some(e) => {
                blocks.push((p, p + 3 + e + 2, gha_expression_after(&w[..e], after_password_name(s, p))));
                from = p + 3 + e + 2;
            }
            None => from = p + 3,
        }
    }
    blocks
}

/// The `${{ … }}` blocks that are not expressions and go whole, wherever they are (after a flag, `Bearer`, in a URL,
/// glued to another block): the ones after a password's or a PIN's name, and the ones holding a credential-shaped
/// literal or word (`${{ 'banana2024' }}`, `${{ secrets.X || 'banana2024' }}`, `${{ password=banana2024 }}`).
fn gha_rejected_ranges(s: &str) -> Vec<(usize, usize)> {
    gha_blocks(s)
        .into_iter()
        .filter(|&(a, e, expression)| !expression && (after_password_name(s, a) || holds_a_credential(&s[a + 3..e - 2])))
        .map(|(a, e, _)| (a, e))
        .collect()
}

/// Whether the content of a block holds a credential-shaped literal (quoted with `'` or `"`) or word.
fn holds_a_credential(c: &str) -> bool {
    let quoted = c.split(['\'', '"']).skip(1).step_by(2).any(gha_secret_literal);
    let words = c.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))).any(|w| !w.contains('.') && gha_secret_literal(w));
    quoted || words
}

/// A literal in an expression that looks like a credential: not a path or a model's name, not a glob (`'**/x.json'`).
fn gha_secret_literal(lit: &str) -> bool {
    literal_secret(lit) && !path_or_model(lit) && !lit.contains('*')
}

/// Whether the `${{` at `p` is the value of a password's or a PIN's name (`password: ${{ … }}`, `PIN=${{ … }}`,
/// `password: "${{ … }}"`, `password: |` and the block on the next line, `my password is ${{ … }}`), read 128 bytes
/// back.
fn after_password_name(s: &str, p: usize) -> bool {
    let before = window_back(s, p, 128);
    let mut lines = before.rsplit('\n');
    // (quotes and spaces between the separator and `${{`: `password: "${{ … }}"`, `password='${{ … }}'`)
    let mut line = lines.next().unwrap_or("").trim_end_matches([' ', '\t', '"', '\'', '`']);
    // (alone on its line, the value of the line before: `password: |` then `  ${{ … }}`)
    if line.is_empty() {
        let prev = lines.next().unwrap_or("").trim_end().trim_end_matches(['-', '+']);
        line = prev.strip_suffix(['|', '>']).map_or(prev, str::trim_end);
    }
    // (a sentence: a verb after the name, `my password is ${{ … }}`)
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.len() >= 2 && PHRASE_VERBS.contains(&words[words.len() - 1].to_lowercase().as_str()) {
        return words[..words.len() - 1].iter().rev().take(3).any(|w| password_name(w.trim_matches(|c: char| !c.is_alphanumeric())));
    }
    let Some(head) = line.strip_suffix([':', '=']).map(|h| h.trim_end().trim_end_matches(['"', '\''])) else { return false };
    let name = head.rsplit(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))).next().unwrap_or("");
    password_name(name)
}

/// A password's or a PIN's name (`password`, `senha`, `db_pass`, `userPin`): an expression after it is checked strictly.
fn password_name(name: &str) -> bool {
    password_label(name) || matches!(suffix_key(name), Some("pin" | "pwd" | "pass" | "pw"))
}

/// A JSON string that only refers to something else (see [`interpolation_only`]); under a password's or a PIN's name,
/// each `${{ … }}` in it is checked strictly.
fn json_reference(t: &str, key: &str) -> bool {
    if !interpolation_only(t) {
        return false;
    }
    if !password_name(key) {
        return true;
    }
    let mut from = 0;
    while let Some(p) = t[from..].find("${{").map(|p| p + from) {
        let Some(e) = t[p + 3..].find("}}").map(|e| p + 3 + e) else { return false };
        if !gha_expression_after(&t[p + 3..e], true) {
            return false;
        }
        from = e + 2;
    }
    true
}

/// At most `n` bytes of `s` before `at` (cut on a character).
fn window_back(s: &str, at: usize, n: usize) -> &str {
    let mut from = at.saturating_sub(n);
    while !s.is_char_boundary(from) {
        from += 1;
    }
    &s[from..at]
}

/// The text with each span written as `*` (the same number of bytes: offsets do not move).
fn mask_spans(s: &str, spans: &[(usize, usize)]) -> String {
    if spans.is_empty() {
        return s.to_string();
    }
    let mut b = s.as_bytes().to_vec();
    for &(a, e) in spans {
        b[a..e].fill(b'*');
    }
    // (a span starts and ends on ASCII, so what is left of the text stays whole characters)
    String::from_utf8(b).unwrap_or_else(|_| s.to_string())
}

/// The ranges with the masked spans taken out of them, and what is left only when it holds a letter or a digit (the
/// quotes around `"${{ … }}"` are not a secret).
fn outside_units(t: &str, ranges: Vec<(usize, usize)>, units: &[(usize, usize)]) -> Vec<(usize, usize)> {
    if units.is_empty() {
        return ranges;
    }
    let mut out = Vec::new();
    for (a, e) in ranges {
        let mut pieces = vec![(a, e)];
        // (the units are in order and do not overlap: only the ones that meet this range are read)
        let first = units.partition_point(|&(_, ue)| ue <= a);
        for &(ua, ue) in units[first..].iter().take_while(|&&(ua, _)| ua < e) {
            pieces = pieces
                .into_iter()
                .flat_map(|(x, y)| if ue <= x || y <= ua { vec![(x, y)] } else { vec![(x, ua.max(x)), (ue.min(y), y)] })
                .filter(|(x, y)| x < y)
                .collect();
        }
        out.extend(pieces.into_iter().filter(|&(x, y)| t.get(x..y).is_some_and(|p| p.chars().any(char::is_alphanumeric))));
    }
    out
}

/// A value of the environment made only of references (`{"TOKEN": "${{ secrets.X }}"}`), which stays.
fn only_references(v: &Value) -> bool {
    references_under(v, "")
}

/// [`only_references`] with the key the value is under (a password's name checks its expressions strictly).
fn references_under(v: &Value, key: &str) -> bool {
    match v {
        Value::String(t) => json_reference(t.trim(), key),
        Value::Object(o) => !o.is_empty() && o.iter().all(|(k, x)| references_under(x, k)),
        Value::Array(a) => !a.is_empty() && a.iter().all(|x| references_under(x, key)),
        _ => false,
    }
}

/// Whether a word of a template's content is a field of a path (`.Values.db.passwordB64`): next to a dot.
fn field_of_path(c: &str, t: &str) -> bool {
    c.match_indices(t).any(|(i, _)| c[..i].ends_with('.') || c[i + t.len()..].starts_with('.'))
}

/// Words before `pass` that make it a ticket ("Boarding pass: seat 14C").
/// Whether the word before `s[st..]` makes a `pass` a ticket ("Boarding pass"). Only 64 bytes back are read: a
/// look-behind that ran to the previous space ran to the start of a text without spaces (JSON, a query string, a
/// cookie) at every `pass`.
fn pass_ticket(s: &str, st: usize) -> bool {
    let mut from = st.saturating_sub(64);
    while !s.is_char_boundary(from) {
        from += 1;
    }
    let before = s[from..st].trim_end_matches([' ', '\t']).rsplit([' ', '\t', '\n']).next().unwrap_or("");
    PASS_TICKETS.contains(&before.to_ascii_lowercase().as_str())
}

const PASS_TICKETS: &[&str] = &["boarding", "bus", "day", "season", "ski", "backstage", "press", "hall", "transit", "museum"];

/// A date in a real layout: `YYYY-MM-DD`, optionally followed by `[T ]HH:MM`, `:SS`, a fraction and a zone (`Z` or
/// `±HH:MM`), each field in its range (month 01–12, day 01–31, hour 00–23, minute and second 00–59).
fn timestamp(t: &str) -> bool {
    let b = t.as_bytes();
    let num = |at: usize, len: usize| -> Option<u32> {
        let d = b.get(at..at + len)?;
        d.iter().all(u8::is_ascii_digit).then(|| d.iter().fold(0, |n, c| n * 10 + u32::from(c - b'0')))
    };
    let in_range = |at: usize, max: u32, min: u32| num(at, 2).is_some_and(|n| (min..=max).contains(&n));
    // the date
    if num(0, 4).is_none() || b.get(4) != Some(&b'-') || !in_range(5, 12, 1) || b.get(7) != Some(&b'-') || !in_range(8, 31, 1) {
        return false;
    }
    if b.len() == 10 {
        return true;
    }
    // the time
    if !matches!(b.get(10), Some(b'T' | b' ')) || !in_range(11, 23, 0) || b.get(13) != Some(&b':') || !in_range(14, 59, 0) {
        return false;
    }
    let mut i = 16;
    if b.get(i) == Some(&b':') {
        if !in_range(i + 1, 59, 0) {
            return false;
        }
        i += 3;
        if b.get(i) == Some(&b'.') {
            let digits = b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
            if digits == 0 {
                return false;
            }
            i += 1 + digits;
        }
    }
    // the zone
    match b.get(i) {
        None => true,
        Some(b'Z') => i + 1 == b.len(),
        Some(b'+' | b'-') => i + 6 == b.len() && in_range(i + 1, 23, 0) && b[i + 3] == b':' && in_range(i + 4, 59, 0),
        _ => false,
    }
}

/// A key about when something happened to a secret (`passwordChangedAt`, `token_updated_at`, `secret_since`): a date
/// under it is about the secret.
fn time_key(key: &str) -> bool {
    key_words(key).last().is_some_and(|w| matches!(w.as_str(), "at" | "date" | "time" | "timestamp" | "changed" | "updated" | "created" | "since"))
}

/// A quoted sentence, not a passphrase: three words of letters or more and a mark of prose, a short word of a language
/// (`o`, `the`, `em`) or punctuation (`O token expira em 24 horas`, `Paste the token here.`); `correct horse battery
/// staple` has none.
fn quoted_prose(q: &str) -> bool {
    let words: Vec<&str> = q.split_whitespace().collect();
    let letters = words.iter().filter(|w| w.chars().all(char::is_alphabetic)).count();
    let marked = words.iter().any(|w| PROSE_WORDS.contains(&fold_accents(w.trim_matches(|c: char| !c.is_alphabetic())).as_str()))
        || q.trim_end().ends_with(['.', '!', '?', ':'])
        || q.contains([',', ';']);
    letters >= 3 && marked
}

/// Whether a value under a key that describes a secret is a description, not the secret: a lifetime only as a
/// duration (`15m`, `3600`, `1h30m`), a namespace only as a plain name (`production`, `my_app`), a date under a key
/// about time, and
/// otherwise a value that is not credential-shaped (beyond the `_` of a name, `client_secret_basic`), not a secret
/// literal and has no digit.
fn describing_value(key: &str, t: &str) -> bool {
    match key_words(key).last().map(String::as_str) {
        Some("lifetime") => return duration(t),
        Some("namespace") => return plain_name(t),
        _ => {}
    }
    (!credential_shaped(&t.replace('_', "")) && !literal_secret(t) && !t.bytes().any(|c| c.is_ascii_digit())) || (time_key(key) && timestamp(t))
}

/// A name whose last word is a password's (`password`, `db_password`, `senha`), not a setting about it
/// (`password_require_symbol`): a word like `true` after it is the password.
fn names_the_password(name: &str) -> bool {
    key_words(name).last().is_some_and(|w| password_name(w))
}

/// A plain name, as a namespace is written (`production`, `my_app`, `MyApp`, `eu-west-1`): it starts with a letter,
/// and each of its parts between `.`, `-` and `_` is only letters or only digits and is not a secret's word
/// (`Prod_Secret`). A part that mixes letters and digits (`ns-4f9a2`, a key in hex) is not plain.
fn plain_name(t: &str) -> bool {
    t.starts_with(|c: char| c.is_ascii_alphabetic())
        && t.split(['.', '-', '_']).all(|p| {
            (p.bytes().all(|c| c.is_ascii_alphabetic()) || p.bytes().all(|c| c.is_ascii_digit()))
                && !SECRET_WORDS.contains(&p.to_ascii_lowercase().as_str())
        })
}

/// Words that name a secret, as a part of a plain name.
const SECRET_WORDS: &[&str] = &["secret", "password", "passwd", "pass", "pwd", "token", "key", "senha", "credential"];

/// A key whose last word is `lifetime` or `namespace`.
fn lifetime_or_namespace(key: &str) -> bool {
    matches!(key_words(key).last().map(String::as_str), Some("lifetime" | "namespace"))
}

/// A duration: a number with an optional unit (`15m`, `3600`, `1h`, `30d`, `500ms`, `7days`), numbers with units in
/// a row (`1h30m`, `168h0m0s`), or ISO 8601 (`PT15M`).
fn duration(t: &str) -> bool {
    let t = t.trim();
    let iso = t
        .strip_prefix("PT")
        .or_else(|| t.strip_prefix('P'))
        .is_some_and(|r| !r.is_empty() && r.bytes().all(|c| c.is_ascii_digit() || matches!(c, b'H' | b'M' | b'S' | b'D' | b'W' | b'Y' | b'T')));
    if iso || (!t.is_empty() && t.bytes().all(|c| c.is_ascii_digit())) {
        return true;
    }
    // (each part a number and a unit)
    let mut rest = t;
    while !rest.is_empty() {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let unit = rest[digits..].bytes().take_while(u8::is_ascii_alphabetic).count();
        if digits == 0 || !DURATION_UNITS.contains(&&rest[digits..digits + unit]) {
            return false;
        }
        rest = &rest[digits + unit..];
    }
    !t.is_empty()
}

/// The units of a [`duration`].
const DURATION_UNITS: &[&str] = &[
    "ns", "us", "ms", "s", "m", "h", "d", "w", "y", "sec", "secs", "min", "mins", "hr", "hrs", "second", "seconds", "minute", "minutes", "hour",
    "hours", "day", "days", "week", "weeks", "year", "years",
];

/// A shell variable, `$VAR` or `${VAR}` with an environment-style name (`$PASSWORD`, `${DB_PASS}`) and nothing else.
fn shell_variable(t: &str) -> bool {
    let name = t.strip_prefix("${").and_then(|r| r.strip_suffix('}')).or_else(|| t.strip_prefix('$'));
    name.is_some_and(|n| {
        !n.is_empty()
            && n.starts_with(|c: char| c.is_ascii_uppercase() || c == '_')
            && n.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
    })
}

/// A key written in hex after a key's label (`api key: 8c1d4e7f…`): sixteen or more hex digits, letters and digits
/// both. (Outside a label, pure hex is a hash or an id and stays.)
fn hex_key(v: &str) -> bool {
    v.len() >= 16 && v.bytes().all(|c| c.is_ascii_hexdigit()) && v.bytes().any(|c| c.is_ascii_digit()) && v.bytes().any(|c| c.is_ascii_alphabetic())
}

/// A mask in place of a value (`***`, `...`, `xxxx`, `••••`): not a secret.
fn mask(t: &str) -> bool {
    t.chars().count() >= 3 && t.chars().all(|c| matches!(c, '*' | 'x' | 'X' | '•' | '.' | '…'))
}

/// A placeholder in angle brackets, words without digits (`<token>`, `<seu token aqui>`, `<your-api-key>`).
fn placeholder(t: &str) -> bool {
    let t = t.trim_end_matches(['.', ',', ';', '`', '\'', '"', ')']);
    t.len() > 2 && t.starts_with('<') && t.ends_with('>') && t[1..t.len() - 1].chars().all(|c| c.is_alphabetic() || matches!(c, ' ' | '-' | '_'))
}

/// Text that names a secret instead of holding one (`"jwt-secret-name"`, `"X-API-Key"`, `"db-credentials"`): one word,
/// not credential-shaped, with a part that is a secret's name.
fn names_a_secret(t: &str) -> bool {
    !t.is_empty()
        && !t.contains(char::is_whitespace)
        && !credential_shaped(t)
        && t.split(['-', '_', '.']).any(|p| {
            let p = p.to_ascii_lowercase();
            matches!(p.as_str(), "key" | "token" | "secret" | "password" | "credentials" | "auth" | "api")
        })
}

/// Whether a key is one of [`TEXT_SECRET_KEYS`], whole or as its last word.
fn text_secret_key(key: &str) -> bool {
    let n = normalize(key);
    TEXT_SECRET_KEYS.contains(&n.as_str()) || key_words(key).last().is_some_and(|w| TEXT_SECRET_KEYS.contains(&w.as_str()))
}

/// A key that spells a password's name with digits or symbols for letters (`passw0rd`, `p@ssword`, `p4ssword`, `s3cret`).
fn leet_password(k: &str) -> bool {
    if !k.bytes().any(|c| matches!(c, b'0' | b'1' | b'3' | b'4' | b'5' | b'@' | b'$')) {
        return false;
    }
    let plain: String = k
        .chars()
        .map(|c| match c {
            '0' => 'o',
            '1' => 'i',
            '3' => 'e',
            '4' | '@' => 'a',
            '5' | '$' => 's',
            c => c,
        })
        .collect();
    ["password", "passwd", "passphrase", "senha", "secret", "token"].iter().any(|w| plain.contains(w))
}

/// A literal in code that looks like a credential and is not a path or a model's name (`"LiquidAI/LFM3-1.7B"`).
fn code_secret(lit: &str) -> bool {
    literal_secret(lit) && !path_or_model(lit)
}

/// A path (`config/v2/token.json`, `secrets/token2024.txt`: segments of a name, the last one a file with an
/// extension) or a model's name (`LiquidAI/LFM3-1.7B`: an organization of letters and a
/// name with a version). A run of base64 with `/` (`Zk9v…/QmF6…+MTIz…==`) is neither.
fn path_or_model(lit: &str) -> bool {
    if !lit.contains('/') {
        return false;
    }
    let segments: Vec<&str> = lit.split('/').collect();
    if !segments.iter().all(|p| p.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-' | b'~'))) {
        return false;
    }
    let file = segments.last().is_some_and(|f| {
        f.rsplit_once('.').is_some_and(|(n, ext)| !n.is_empty() && (1..=5).contains(&ext.len()) && ext.bytes().all(|c| c.is_ascii_alphabetic()))
    });
    let model = segments.len() == 2
        && segments[0].len() >= 2
        && segments[0].bytes().all(|c| c.is_ascii_alphabetic() || c == b'-')
        && segments[1].bytes().any(|c| c.is_ascii_alphabetic())
        && segments[1].contains(['-', '.']);
    // (a path that starts with `/`, `./` or `~/` is never a literal secret: see `literal_secret`)
    file || model
}

/// A literal in code that looks like a credential: one word of four or more characters with letters and a digit, a
/// symbol or a case change inside, and not a name (an identifier without digits, an environment variable's name), a
/// path, a URL, a format or a template.
fn literal_secret(lit: &str) -> bool {
    let b = lit.as_bytes();
    let has = |f: fn(&u8) -> bool| b.iter().any(f);
    let identifier = lit.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_');
    lit.len() >= 4
        && !lit.contains(char::is_whitespace)
        && !lit.contains("://")
        && !lit.starts_with(['/', '.', '-', '$', '~'])
        && !lit.contains(['{', '%', '\\'])
        && !b.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == b'_')
        && has(u8::is_ascii_alphabetic)
        && !(identifier && !has(u8::is_ascii_digit))
        // (an id or a hash in hex, a UUID: `"traceId": "0af7…"`; the token detector says the same)
        && !b.iter().all(|c| c.is_ascii_hexdigit() || *c == b'-')
        // (a selector or a name with a secret's name in it and no digit: `#password`, `.api-key`)
        && !(!has(u8::is_ascii_digit) && ["password", "token", "secret", "key", "senha"].iter().any(|w| lit.to_ascii_lowercase().contains(w)))
        && (has(u8::is_ascii_digit) || has(|c| matches!(c, b'!' | b'@' | b'#' | b'$' | b'^' | b'&' | b'*' | b'+' | b'=' | b'~')))
}

/// `Bearer X`, `Basic X`, `Token X`, `ApiKey X` outside a label: the word after the scheme goes when it looks like a
/// credential; a placeholder (`<token>`), a variable, a parameter (`realm="api"`) or a common word stays.
fn scheme_ranges(s: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let lower = s.to_ascii_lowercase();
    if !AUTH_SCHEMES.iter().any(|w| lower.contains(w)) {
        return ranges;
    }
    let words: Vec<(usize, &str)> =
        s.split(char::is_whitespace).filter(|w| !w.is_empty()).map(|w| (w.as_ptr() as usize - s.as_ptr() as usize, w)).collect();
    for pair in words.windows(2) {
        let scheme = pair[0].1.trim_start_matches(['"', '\'', '`', '(']).trim_end_matches(':').to_ascii_lowercase();
        if !AUTH_SCHEMES.contains(&scheme.as_str()) {
            continue;
        }
        let (at, next) = pair[1];
        let lead = next.len() - next.trim_start_matches(['"', '\'', '`']).len();
        // (the credential's own characters: a quote, a bracket or punctuation after it is not part of it)
        let run = next[lead..]
            .bytes()
            .take_while(|c| {
                c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'=' | b'-' | b'_' | b'.' | b'~' | b'$' | b'{' | b'}' | b'<' | b'>')
            })
            .count();
        let v = next[lead..lead + run].trim_end_matches(['.', ',']);
        let body = v.trim_end_matches('=');
        // (a parameter, `realm="api"`: something follows its `=`; base64 padding ends the word)
        let parameter = next[lead..].find('=').is_some_and(|p| !next[lead + p..].trim_start_matches('=').is_empty());
        if v.is_empty()
            || v.starts_with("[REDACTED")
            || placeholder(v)
            || interpolation_at(v).is_some()
            || !value_is_literal(v, "")
            || body.contains('=')
            || parameter
        {
            continue;
        }
        let letters_and_digits = v.bytes().any(|c| c.is_ascii_alphabetic()) && v.bytes().any(|c| c.is_ascii_digit());
        let opaque = body.len() >= 16 && body.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'-' | b'_' | b'.'));
        let same_line = !s[pair[0].0..at].contains('\n');
        let looks = letters_and_digits || opaque || (scheme != "token" && credential_shaped(v) && !v.contains('_'));
        if looks && same_line {
            ranges.push((at + lead, at + lead + v.len()));
        }
    }
    ranges
}

/// The text of a JSON document with only the values that its redacted copy changed written again, where they were: the
/// layout and the order of the keys stay. `None` when the two do not line up (the caller writes the copy).
fn patch_json(text: &str, after: &Value) -> Option<String> {
    let b = text.as_bytes();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let end = json_walk(text, 0, Some(after), &mut edits)?;
    if json_ws(b, end) != b.len() {
        return None;
    }
    edits.sort_by_key(|e| e.0);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (a, e, with) in edits {
        if a < at {
            return None;
        }
        out.push_str(&text[at..a]);
        out.push_str(&with);
        at = e;
    }
    out.push_str(&text[at..]);
    Some(out)
}

fn json_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

/// Where the JSON string that starts at `i` (its quote) ends, past its closing quote.
fn json_string_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            b'"' => return Some(j + 1),
            _ => j += 1,
        }
    }
    None
}

/// Walks the JSON value at `i` next to its redacted copy (`None`: only skips it), noting each value of the text that
/// the copy changed; returns where the value ends.
fn json_walk(t: &str, i: usize, after: Option<&Value>, edits: &mut Vec<(usize, usize, String)>) -> Option<usize> {
    let b = t.as_bytes();
    let i = json_ws(b, i);
    let replaced_whole = |edits: &mut Vec<(usize, usize, String)>, a: &Value| -> Option<usize> {
        let end = json_walk(t, i, None, edits)?;
        edits.push((i, end, a.to_string()));
        Some(end)
    };
    match *b.get(i)? {
        b'{' => {
            if let Some(a) = after.filter(|a| !a.is_object()) {
                return replaced_whole(edits, a);
            }
            let obj = after.and_then(Value::as_object);
            let mut j = json_ws(b, i + 1);
            if b.get(j) == Some(&b'}') {
                return Some(j + 1);
            }
            loop {
                if b.get(j) != Some(&b'"') {
                    return None;
                }
                let ke = json_string_end(b, j)?;
                let key: String = serde_json::from_str(&t[j..ke]).ok()?;
                // (a key that is a token is redacted too, as `redact_value` does)
                let new_key = tokens(&key, &mut Stats::default());
                let child = match obj {
                    Some(o) => {
                        let (k, v) = match o.get(&new_key) {
                            Some(v) => (new_key.clone(), v),
                            None => (key.clone(), o.get(&key)?),
                        };
                        if k != key {
                            edits.push((j, ke, Value::String(k).to_string()));
                        }
                        Some(v)
                    }
                    None => None,
                };
                j = json_ws(b, ke);
                if b.get(j) != Some(&b':') {
                    return None;
                }
                j = json_ws(b, json_walk(t, j + 1, child, edits)?);
                match *b.get(j)? {
                    b',' => j = json_ws(b, j + 1),
                    b'}' => return Some(j + 1),
                    _ => return None,
                }
            }
        }
        b'[' => {
            if let Some(a) = after.filter(|a| !a.is_array()) {
                return replaced_whole(edits, a);
            }
            let arr = after.and_then(Value::as_array);
            let mut j = json_ws(b, i + 1);
            if b.get(j) == Some(&b']') {
                return Some(j + 1);
            }
            let mut n = 0;
            loop {
                let child = match arr {
                    Some(a) => Some(a.get(n)?),
                    None => None,
                };
                j = json_ws(b, json_walk(t, j, child, edits)?);
                n += 1;
                match *b.get(j)? {
                    b',' => j += 1,
                    b']' => return Some(j + 1),
                    _ => return None,
                }
            }
        }
        b'"' => {
            let e = json_string_end(b, i)?;
            if let Some(a) = after {
                let old: String = serde_json::from_str(&t[i..e]).ok()?;
                if a.as_str() != Some(old.as_str()) {
                    edits.push((i, e, a.to_string()));
                }
            }
            Some(e)
        }
        _ => {
            // a number, `true`, `false` or `null`
            let mut e = i;
            while e < b.len() && !matches!(b[e], b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r') {
                e += 1;
            }
            if let Some(a) = after {
                let old: Value = serde_json::from_str(&t[i..e]).ok()?;
                if &old != a {
                    edits.push((i, e, a.to_string()));
                }
            }
            Some(e)
        }
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
            match auth[..at].find(':') {
                Some(colon) if colon + 1 < at && !interpolation_only(&auth[colon + 1..at]) => ranges.push((start + colon + 1, start + at)),
                // (a token as the whole user info: `https://X@host`; `git@`, `ana@` are users)
                None if literal_secret(&auth[..at]) => ranges.push((start, start + at)),
                _ => {}
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
    // (`export` lines are a script's configuration, not a dump of the environment: they do not count, and the names of
    // secrets among them are read as pairs)
    if s.lines().filter(|l| !l.starts_with("export ") && is_env(l)).count() < 3 {
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
        b':' | b'?' if b.get(i + 1) == Some(&b'=') => Some((2, false)),
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

/// Authorization schemes: the word after them is the credential.
const AUTH_SCHEMES: &[&str] = &["bearer", "basic", "token", "apikey", "negotiate", "ntlm"];

/// Parameters of a URL that carry a signature or a key (`?sig=`, `&key=`), when their value looks like a credential.
const QUERY_SECRET_NAMES: &[&str] = &["sig", "signature", "key", "code", "auth", "access_token", "id_token", "apikey"];

/// Names too common to be a secret by themselves (`key: value`): a secret only before a credential-shaped value.
const GENERIC_SECRET_NAMES: &[&str] = &["key", "chave", "clave", "cle", "schlussel", "chiave"];

/// Common words that follow `token` in a log line ("auth: token refreshed"): not a credential after a scheme.
const COMMON_AFTER_SCHEME: &[&str] = &[
    "refreshed",
    "expired",
    "missing",
    "invalid",
    "required",
    "revoked",
    "rotated",
    "renewed",
    "created",
    "deleted",
    "issued",
    "granted",
    "denied",
    "rejected",
    "accepted",
    "failed",
    "verified",
    "validated",
    "received",
    "sent",
    "saved",
    "stored",
    "updated",
    "generated",
    "found",
    "error",
    "mismatch",
    "unknown",
    "empty",
    "none",
    "null",
    "set",
    "unset",
    "reset",
    "ok",
    "not",
    "is",
    "was",
    "has",
    "for",
    "from",
    "cache",
    "expirado",
    "invalido",
    "renovado",
    "ausente",
    "recebido",
    "salvo",
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
    // the value and what follows it on its line: a shape of code must be whole there
    // (at most 256 bytes ahead: a value is judged in one pass, whatever the length of the text)
    let mut ahead = rest.len().min(256);
    while !rest.is_char_boundary(ahead) {
        ahead -= 1;
    }
    let line = format!("{value}{}", rest[..ahead].split('\n').next().unwrap_or(""));
    // (an expression of GitHub Actions cut by the end of the line, `${{ secrets.X`, is a reference; a closed one that is
    // still in the text was not masked as one: it holds a literal, and is one)
    if value.starts_with("${{") {
        return line.contains("}}");
    }
    // a structure that opens here and goes on (`auth: { user: …`, `tokens: [`)
    // (or a remark in parentheses on its line, several words: "Senha: (deixe em branco)")
    if matches!(value, "{" | "[" | "(") || (line.starts_with('(') && closes(&line) && line[1..line.find(')').unwrap_or(1)].contains(' ')) {
        return false;
    }
    // a whole interpolation (`${VAR}`, `$(cat file)`, `{{ x }}`, `$VAR`) and nothing glued after it
    if let Some(n) = interpolation_at(&line) {
        if n >= value.len() {
            return false;
        }
    }
    // a placeholder in angle brackets: words without digits (`<seu token aqui>`, `<your-api-key>`)
    if let Some(inner) = line.strip_prefix('<').and_then(|l| l.split_once('>')).map(|(a, _)| a) {
        if !inner.trim().is_empty() && inner.chars().all(|c| c.is_alphabetic() || matches!(c, ' ' | '-' | '_')) {
            return false;
        }
    }
    // a positional placeholder of SQL (`$1`, `$2`)
    if value.len() > 1 && value.starts_with('$') && value[1..].bytes().all(|c| c.is_ascii_digit()) {
        return false;
    }
    // `%VAR%`, and a format placeholder: `%s`, `%(name)s`
    if (value.len() > 2 && value.starts_with('%') && value.ends_with('%') && variable_name(&value[1..value.len() - 1]))
        || (value.starts_with('%') && value[1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '('))
    {
        return false;
    }
    if value.len() >= 3 && value.chars().all(|c| matches!(c, '*' | 'x' | 'X' | '•' | '.')) {
        return false;
    }
    // a call or an index on an identifier path (`lexer.next_token()`, `os.environ["X"]`, `std::env::var("X")`),
    // its bracket closed on the line
    let ident_path = |p: &str| {
        !p.is_empty()
            && p.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '$')
            && p.bytes().any(|c| c.is_ascii_alphabetic())
            && !p.contains(":::")
            && p.split("::").all(|seg| !seg.is_empty() && seg.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'$' | b'?')))
    };
    if let Some(at) = line.find(['(', '[']) {
        // (a macro is called with `!`: `format!(…)`)
        if at <= value.len() && ident_path(line[..at].strip_suffix('!').unwrap_or(&line[..at])) && closes(&line[at..]) {
            return false;
        }
    }
    // a field of a configuration object: `os.environ`, `process.env.API_KEY`, `settings.SECRET_KEY`
    if let Some((first, more)) = value.split_once('.') {
        if !more.is_empty() && !value.contains("::") && ident_path(value) && CONFIG_OBJECTS.contains(&first.to_ascii_lowercase().as_str()) {
            return false;
        }
    }
    true
}

/// A name of a variable: an identifier without digits (`DB_PASSWORD`, `token`, `apiKey`); `banana2024` is not one.
fn variable_name(t: &str) -> bool {
    !t.is_empty() && t.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') && t.bytes().all(|c| c.is_ascii_alphabetic() || c == b'_')
}

/// What `${…}`, `{…}` or `{{…}}` hold when they interpolate: variables, fields, indexes and defaults
/// (`secrets.GITHUB_TOKEN`, `JWT_SECRET:-`, `token[:4]`), never a credential-shaped word.
fn expression(c: &str) -> bool {
    let c = c.trim();
    !c.is_empty()
        && c.chars().all(|ch| ch.is_ascii_alphanumeric() || " _.:-[]'\"|()!?,".contains(ch))
        && c.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
            .filter(|t| !t.is_empty())
            .all(|t| variable_name(t) || t.bytes().all(|b| b.is_ascii_digit()) || field_of_path(c, t))
}

/// What `$(…)` holds when it reads a secret from somewhere: a command named by a word without digits (`cat`, `pass`,
/// `vault`); `echo` and `printf` would print the secret itself.
fn command_substitution(c: &str) -> bool {
    c.split_whitespace().next().is_some_and(|w| variable_name(w) && !matches!(w, "echo" | "printf" | "print"))
}

/// The length of the interpolation that starts `v` (`${VAR}`, `$(cmd)`, `{{ x }}`, `{x}`, `$VAR`), when it is whole
/// and holds a variable, an expression or a command.
fn interpolation_at(v: &str) -> Option<usize> {
    if let Some(name) = v.strip_prefix('$').filter(|n| !n.starts_with(['{', '('])) {
        let n = name.bytes().take_while(|c| c.is_ascii_alphanumeric() || *c == b'_').count();
        return variable_name(&name[..n]).then_some(1 + n);
    }
    // (`${{ … }}` is a GitHub Actions expression, one unit up to its `}}`, whatever it holds)
    let (open, close, ok): (&str, &str, fn(&str) -> bool) = if v.starts_with("${{") {
        ("${{", "}}", gha_expression)
    } else if v.starts_with("{{") {
        ("{{", "}}", expression)
    } else if v.starts_with("${") {
        ("${", "}", expression)
    } else if v.starts_with("$(") {
        ("$(", ")", command_substitution)
    } else if v.starts_with('{') {
        ("{", "}", expression)
    } else {
        return None;
    };
    let e = v[open.len()..].find(close)?;
    ok(&v[open.len()..open.len() + e]).then_some(open.len() + e + close.len())
}

/// A quoted value that only interpolates something else (`"${DB_PASSWORD}"`, `` `Bearer ${token}` ``, `"{{ secret }}"`):
/// every interpolation whole and holding a variable or an expression, and nothing else but a scheme word. When in
/// doubt (`"$banana2024"`, `"{banana2024}"`) it is a literal.
fn interpolation_only(v: &str) -> bool {
    let b = v.as_bytes();
    let (mut rest, mut found, mut i) = (String::new(), false, 0);
    while i < b.len() {
        if matches!(b[i], b'$' | b'{') {
            match interpolation_at(&v[i..]) {
                Some(n) => {
                    found = true;
                    i += n;
                    continue;
                }
                None => return false,
            }
        }
        let n = v[i..].chars().next().map_or(1, char::len_utf8);
        rest.push_str(&v[i..i + n]);
        i += n;
    }
    found && rest.split_whitespace().all(|w| matches!(w.to_ascii_lowercase().as_str(), "bearer" | "basic" | "token"))
}

/// Whether the bracket at the start of `t` closes on its line.
fn closes(t: &str) -> bool {
    let (open, close) = match t.as_bytes().first() {
        Some(b'(') => (b'(', b')'),
        Some(b'[') => (b'[', b']'),
        _ => return false,
    };
    let mut depth = 0usize;
    for &c in t.as_bytes() {
        if c == b'\n' {
            return false;
        }
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return true;
            }
        }
    }
    false
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
    "tokendeacesso",
    "chavedeapi",
    "chavedaapi",
    "chavedeacesso",
    "chaveprivada",
    "chavesecreta",
    "codigodeacesso",
    "clavedeacceso",
    "codicediaccesso",
    "passphrase",
    "password",
    "seedphrase",
    "recoverycode",
    "backupcodes",
    "accesscode",
    "wifikey",
    "apppassword",
    "bearertoken",
    "sharedsecret",
];

/// Words after a password's name that say which one it is ("senha nova:", "senha atual X", "root password X").
const LABEL_MODIFIERS: &[&str] = &[
    "nova",
    "novo",
    "atual",
    "antiga",
    "antigo",
    "temporaria",
    "temporario",
    "provisoria",
    "padrao",
    "mestra",
    "new",
    "old",
    "current",
    "temporary",
    "default",
    "master",
    "root",
    "admin",
    "app",
    "wifi",
    "wi-fi",
    "nueva",
    "actual",
    "nouveau",
    "nouvelle",
    "neues",
    "aktuelles",
    "nuova",
    "attuale",
];

/// Prepositions between a label and what it is for ("senha de acesso", "password for the server").
const LABEL_PREPOSITIONS: &[&str] = &[
    "de", "do", "da", "dos", "das", "of", "for", "para", "del", "du", "des", "für", "fur", "di", "della", "dello", "pour", "zum", "zur", "для", "от",
];

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
    // "senha nova:", "root password:" (a password's name and the word that says which one)
    let (last, before) = (fold_accents(words[n - 1]), words[n - 2]);
    if password_label(before) && normalize(before).len() > 3 && LABEL_MODIFIERS.contains(&last.as_str()) {
        return Some(true);
    }
    // "senha de acesso:", "password for the server:", "пароль для входа:" (as often a sentence: see the caller)
    for k in 3..=n {
        let (label, prep) = (words[n - k], fold_accents(words[n - k + 1]));
        if password_label(label) && normalize(label).len() > 3 && LABEL_PREPOSITIONS.contains(&prep.as_str()) {
            return Some(false);
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
    "na",
    "nas",
    "nos",
    "num",
    "numa",
    "pela",
    "pelo",
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
    "y",
    "et",
    "und",
    "ed",
    "debe",
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
    "aceita",
    "aceito",
    "obligatorio",
    "requerido",
    "necesario",
    "incorrecto",
    "valido",
    "invalido",
    "corto",
    "largo",
    "opcional",
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
    "mdp",
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
    "codice di accesso",
    "codigo de acesso",
    "access code",
    "wifi key",
    "wi-fi key",
    "pass",
    "psk",
];

/// Verbs that are also ordinary words ("e" is "and", "to", "para"): the value after them goes only when it looks like a
/// credential ("minha senha e banana2024", "set the password to banana2024").
const PHRASE_WEAK_VERBS: &[&str] = &["e", "to", "para", "ficou", "becomes", "agora", "|", "-", "→", "->", ">", "=>"];

/// Words after a password's name that make a compound about it ("password manager is Bitwarden").
const PASSWORD_COMPOUNDS: &[&str] = &[
    "manager",
    "managers",
    "reset",
    "policy",
    "hash",
    "hashing",
    "field",
    "prompt",
    "strength",
    "generator",
    "vault",
    "store",
    "rules",
    "length",
    "expiry",
    "change",
    "recovery",
    "hint",
    "input",
    "box",
    "dialog",
];

/// Names that take a credential-shaped word right after them with no verb ("root password X", "pass X" in a netrc).
const VERBLESS_EXTRA: &[&str] = &["pass", "psk"];
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
    // (the names of secrets, normalized once: a value that is one is the variable)
    let known: Vec<String> = DENY_KEYS.iter().chain(PHRASE_KEYS).map(|d| normalize(d)).collect();
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
        // (a compound about the password, "password manager", "password reset", is not the password)
        if cleaned.get(after_name).is_some_and(|w| PASSWORD_COMPOUNDS.contains(&w.as_str())) {
            k = after_name;
            continue;
        }
        // a prompt answered on its line: `Password for ana@host: X`, `[sudo] password for ana: X` (the answer, whole)
        if strong && cleaned.get(after_name).is_some_and(|w| matches!(w.as_str(), "for" | "de" | "para" | "для")) {
            if let (Some(who), Some(answer)) = (words.get(after_name + 1), words.get(after_name + 2)) {
                if who.0.ends_with(':') && !s[who.2..answer.1].contains('\n') && !answer.0.starts_with("[REDACTED") {
                    ranges.push((answer.1, answer.2));
                    k = after_name + 3;
                    continue;
                }
            }
        }
        // (a name that ends its clause, `{ password, …rest }`, is not followed by its value)
        if words[after_name - 1].0.ends_with([',', ';', ')']) {
            k += 1;
            continue;
        }
        // a verb within four words (not past code, not after a negation); a weak verb only before a credential
        // (a credential on the same line as the name: letters with a digit or a symbol, not a name or a number)
        let label_end = words[after_name - 1].2;
        let shaped_at = |j: usize| {
            words.get(j).is_some_and(|w| {
                let t = w.0.trim_end_matches(['.', ',', ';', '!', '?']);
                !s[label_end..w.1].contains('\n') && literal_secret(t) && value_is_literal(t, "")
            })
        };
        let mut verb = None;
        let mut weak_verb = false;
        for (j, &(w, _, _)) in words.iter().enumerate().take(after_name + 4).skip(after_name) {
            // (not past code, nor past the end of the name's line)
            if w.contains(['{', '}', '(', ')', ';']) || s[label_end..words[j].1].contains('\n') {
                break;
            }
            let lw = w.trim_end_matches([':', ',']).to_lowercase();
            let strong_verb = PHRASE_VERBS.contains(&lw.as_str()) || PHRASE_VERBS.contains(&w);
            let weak = PHRASE_WEAK_VERBS.contains(&lw.as_str());
            if !strong_verb && !weak {
                continue;
            }
            if strong_verb || shaped_at(j + 1) {
                verb = Some(j);
                weak_verb = !strong_verb;
                break;
            }
        }
        // no verb: a credential-shaped word right after a password's name, or after the words that say which one
        // ("root password X", "senha atual X", "пароль от почты X")
        let verbless = || {
            if !(strong || VERBLESS_EXTRA.contains(&cleaned[k].as_str()) || non_latin) || words[after_name - 1].0.ends_with([':', '.']) {
                return None;
            }
            let mut j = after_name;
            let mut skipped = 0;
            while j < words.len() && skipped <= 2 {
                // (a word with a colon before another value on its line is who the password is for: `admin: X`)
                // (or, after a preposition, alone: "Password for ana@host:" is a prompt not yet answered)
                let before_value = words.get(j + 1).is_some_and(|n| !s[words[j].2..n.1].contains('\n'));
                let after_preposition = j > after_name && LABEL_PREPOSITIONS.contains(&cleaned[j - 1].as_str());
                let who = words[j].0.ends_with(':') && (before_value || after_preposition);
                if who {
                    j += 1;
                    skipped += 1;
                    continue;
                }
                if shaped_at(j) && !cleaned[j].is_empty() {
                    return Some(j);
                }
                let w = cleaned[j].as_str();
                let after_preposition = skipped > 0 && LABEL_PREPOSITIONS.contains(&cleaned[j - 1].as_str());
                if !(LABEL_MODIFIERS.contains(&w) || LABEL_PREPOSITIONS.contains(&w) || after_preposition)
                    || words[j].0.ends_with([',', ';', ':', '.'])
                {
                    return None;
                }
                j += 1;
                skipped += 1;
            }
            None
        };
        let mut j = match verb {
            Some(v) => v + 1,
            None => match verbless() {
                Some(j) => {
                    weak_verb = true;
                    j
                }
                None => {
                    k += 1;
                    continue;
                }
            },
        };
        let strong = strong && !weak_verb;
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
                let quoted = &s[inner..c];
                if c > inner
                    && !quoted.contains('\n')
                    && !quoted.starts_with("[REDACTED")
                    && !interpolation_only(quoted)
                    && !placeholder(quoted)
                    && !mask(quoted)
                    && (strong || credential_shaped(quoted))
                {
                    ranges.push((inner, c));
                }
                k = j + 1;
                continue;
            }
        }
        let value = raw.trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '(' | ')' | '«' | '»' | '“' | '”'));
        let value = value.trim_end_matches([',', ';', '.', ')', '!', '?', ':']);
        let offset = raw.find(value).unwrap_or(0);
        let (vs, ve) = (ws + offset, ws + offset + value.len());
        // (a value that is the name of a secret, `self.password = password`, is the variable)
        let named = !value.is_empty() && known.contains(&normalize(value));
        if value.is_empty()
            || value.starts_with("[REDACTED")
            || named
            || !value_is_literal(value, &s[ve.min(we)..])
            || (!strong && !credential_shaped(value))
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
    for (n, line) in lines.iter().enumerate() {
        // (the value on the next line, or after blank lines and a heading's underline: `# Senha\n\nX`, `Senha\n-----\nX`)
        let Some(next) = lines[n + 1..].iter().take(3).find(|(_, l)| !l.trim().is_empty() && !l.trim().chars().all(|c| matches!(c, '-' | '=')))
        else {
            continue;
        };
        let pair = [*line, *next];
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
                .take_while(|c| matches!(c, ' ' | '\t' | ':' | '=' | '是' | '为' | '為' | 'は' | 'が' | '：' | '는' | '은' | '「' | '『' | '“'))
                .map(char::len_utf8)
                .sum();
            let vs = from + skip;
            let len: usize = s[vs..]
                .chars()
                .take_while(|c| c.is_ascii_graphic() && !matches!(c, '"' | '\'' | '`' | ')' | '}' | ']' | '>' | ',' | ';'))
                .map(char::len_utf8)
                .sum();
            let value = s[vs..vs + len].trim_end_matches(['.', ',', ';', '!', '?', ')']);
            // ("パスワードは8文字以上": a number of characters, not a password)
            if value.chars().count() >= 4 && !value.starts_with("[REDACTED") {
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

    /// A JSON key whose last word describes the secret (`password_hint`, `api_key_env`) still loses a string value, and
    /// the final scan flags it when it is left; only a number or a boolean under it stays (`password_min_length: 12`).
    #[test]
    fn keys_that_describe_a_secret_fail_closed() {
        let pw = weak_pw();
        let r = Redactor::default();
        for key in [
            "password_hint",
            "token_prompt",
            "secret_field",
            "api_key_env",
            "password_stdin",
            "token_var",
            "password_required",
            "secret_label",
            "token_endpoint",
            "password_policy",
        ] {
            let v = json!({ key: pw });
            assert_eq!(r.redact_value(&v, &mut Stats::default())[key], REDACTED, "{key}");
            assert_eq!(r.scan_value(&v), 1, "the final scan flags {key}");
            assert_gone(&v.to_string(), &pw);
        }
        // in text, only the descriptions of a secret that cannot hold one stay
        for t in [format!("password_hint: {pw}"), format!("api_key_env={pw}"), format!("token_prompt: {pw}"), format!("app --password-stdin {pw}")] {
            assert_gone(&t, &pw);
        }
        let counts = json!({ "password_min_length": 12, "token_ttl": 3600, "password_required": true });
        assert_eq!(r.redact_value(&counts, &mut Stats::default()), counts);
        assert_eq!(r.scan_value(&counts), 0);
    }

    /// A quoted value is an interpolation only when what it interpolates is a variable or an expression, not a
    /// credential (`"$banana2024"`, `"{banana2024}"` go; `"${DB_PASSWORD}"`, `f'Bearer {token}'` stay).
    #[test]
    fn interpolation_needs_a_variable() {
        let pw = weak_pw();
        for t in [
            format!("password = \"${pw}\""),
            format!("token: \"{{{pw}}}\""),
            format!("secret: \"${{{pw}}}\""),
            format!("api_key: \"{{{{ {pw} }}}}\""),
            format!("token: 'Bearer {{{pw}}}'"),
            format!("password: \"$({pw})\""),
        ] {
            assert_gone(&t, &pw);
        }
        for ok in [
            "password: \"${DB_PASSWORD}\"",
            "token: \"{{ secrets.GITHUB_TOKEN }}\"",
            "session.headers.update({'Authorization': f'Bearer {token}'})",
            "password = \"$DB_PASSWORD\"",
            "secret: \"${JWT_SECRET:-}\"",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// An unquoted value is code only when the whole of it is code: a balanced call or index on an identifier path, a
    /// complete `${VAR}` or `$(command)`, a Rust path, a placeholder in angle brackets, a structure that opens there.
    #[test]
    fn code_shapes_must_be_whole() {
        let pw = weak_pw();
        for t in [
            format!("password={{{pw}"),
            format!("token=[{pw}]"),
            format!("password=({pw})"),
            format!("secret=<{pw}>"),
            format!("password=${{{pw}"),
            format!("token={{{{{pw}"),
            format!("password=abc::{pw}"),
            format!("password=$(echo {pw})"),
            format!("token=f({pw}"),
        ] {
            assert_gone(&t, &pw);
        }
        for ok in [
            "auth: { user: process.env.SMTP_USER, pass: process.env.SMTP_PASS },",
            "let password = std::env::var(\"DB_PASSWORD\")?;",
            "token: <seu token aqui>",
            "export GITHUB_TOKEN=${GITHUB_TOKEN:-}",
            "TOKEN=$(cat ~/.config/token)",
            "token = lexer.next_token()",
            "API_KEY = os.environ[\"API_KEY\"]",
            "cookie: { secure: true, httpOnly: true },",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// After a scheme (`Bearer`, `Basic`, `Token`) under a secret's name the next word goes, whatever its shape; a log
    /// line where the scheme word is followed by a common word ("token refreshed") stays.
    #[test]
    fn the_word_after_a_scheme_goes() {
        let word = concat!("hunter", "pass");
        for t in [
            format!("X-Auth: Bearer {word}"),
            format!("token: Bearer {word}"),
            format!("api_key: Token {word}"),
            format!("Authorization: Basic {word}"),
        ] {
            assert_gone(&t, word);
        }
        for ok in ["2026-11-26T10:12:03Z INFO auth: token refreshed for user=ana", "auth: token expired", "WARN auth: token missing", "token: Bearer"]
        {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// The word after `Bearer`, `Basic`, `Token`, `ApiKey` or `Negotiate` goes outside a label too, when it looks like a
    /// credential; a placeholder, a variable, a parameter or a common word after the scheme stays.
    #[test]
    fn scheme_words_outside_a_label_go() {
        let pw = weak_pw();
        for t in [
            format!("Bearer {pw}"),
            format!("send Bearer {pw} now"),
            format!("Basic {pw}"),
            format!("Token {pw}"),
            format!("header is Bearer {pw}"),
            format!("Bearer: {pw}"),
            format!("Authorization: Bearer: {pw}"),
            format!("Authorization: ApiKey {pw}"),
            format!("Authorization: Negotiate {pw}"),
            format!("Authorization: Digest username=\"a\", response=\"{pw}\""),
        ] {
            assert_gone(&t, &pw);
        }
        assert_eq!(red(&format!("headers = {{'Authorization': 'Bearer {pw}'}}")), format!("headers = {{'Authorization': '{REDACTED}'}}"));
        assert_eq!(red(&format!("Authorization: Basic {pw}=")), format!("Authorization: {REDACTED}"));
        let doc = "The `Authorization` header must carry a bearer token. Example: `Authorization: Bearer <token>`.\nNever log the token.";
        for ok in [
            doc,
            "WWW-Authenticate: Bearer realm=\"api\"",
            "a bearer token is required",
            "O token do modelo tem 4096 posições.",
            "Bearer ${token}",
            "the token 4096",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// Every pair of a `Cookie` or `Set-Cookie` header loses its value; the attributes of `Set-Cookie` stay.
    #[test]
    fn every_cookie_pair_goes() {
        let pw = weak_pw();
        for t in [
            format!("Cookie: theme=dark; session={pw}"),
            format!("Cookie: a=1; b=2; sid={pw}"),
            format!("cookie: _ga=GA1.2.123; sessionid={pw}"),
            format!("Set-Cookie: theme=dark; sessionid={pw}; Path=/"),
        ] {
            assert_gone(&t, &pw);
        }
        assert_eq!(red(&format!("Set-Cookie: sessionid={pw}; Path=/; HttpOnly")), format!("Set-Cookie: sessionid={REDACTED}; Path=/; HttpOnly"));
    }

    /// More ways to say what the password is: without accents (`senha e X`), with `to`/`para`/`ficou`, with no verb
    /// before a credential-shaped word (`root password X`, netrc, ssh config), and more names (`mdp`, `codice di accesso`).
    #[test]
    fn more_ways_to_say_the_password_go() {
        let pw = weak_pw();
        for t in [
            format!("minha senha e {pw}"),
            format!("la password e {pw}"),
            format!("set the password to {pw}"),
            format!("changed the password to {pw}"),
            format!("mudei a senha para {pw}"),
            format!("use password {pw} to log in"),
            format!("use a senha {pw} para entrar"),
            format!("root password {pw}"),
            format!("a senha ficou {pw}"),
            format!("senha atual {pw}"),
            format!("пароль от почты {pw}"),
            format!("il codice di accesso è {pw}"),
            format!("mon mdp est {pw}"),
            format!("The wifi key is {pw}"),
            format!("Wi-Fi: casa, senha {pw}"),
            format!("machine x login ana password {pw}"),
            format!("Host x\n  User a\n  Password {pw}\n"),
            format!("user ana pass {pw}"),
        ] {
            assert_gone(&t, &pw);
        }
        for ok in [
            "a senha e o usuário são obrigatórios",
            "My password manager flagged my old password as weak, so I changed the password to something longer.",
            "Decidimos trocar a senha do servidor de homologação até sexta.",
            "Hello, can you explain how password hashing with argon2 works?",
            "a senha para entrar é obrigatória",
            "A senha deve ter ao menos 8 caracteres.",
            "pass show email/work",
            "my password manager is Bitwarden",
            "password = password",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// Labels of several words in Portuguese and Russian, other spellings of the names, and `=>`.
    #[test]
    fn compound_labels_and_spellings_go() {
        let pw = weak_pw();
        // (a password's name and the word that says which one take a passphrase too)
        assert_gone(concat!("senha nova: bolo de ", "cenoura"), "cenoura");
        for t in [
            format!("senha nova: {pw}"),
            format!("token de acesso: {pw}"),
            format!("chave de API: {pw}"),
            format!("chave privada: {pw}"),
            format!("пароль для входа: {pw}"),
            format!("pass phrase: {pw}"),
            format!("pass word: {pw}"),
            format!("seed phrase: {pw}"),
            format!("recovery code: {pw}"),
            format!("passw0rd: {pw}"),
            format!("p@ssword: {pw}"),
            format!("p4ssword: {pw}"),
            format!("creds: {pw}"),
            format!("passcode: {pw}"),
            format!("pswd: {pw}"),
            format!("psk={pw}"),
            format!("'password' => '{pw}'"),
            format!("\"password\" => \"{pw}\""),
            format!("password => {pw}"),
        ] {
            assert_gone(&t, &pw);
        }
        for ok in [
            "Veja a seção de segurança para saber como guardamos a senha do usuário: usamos Argon2id com salt de 16 bytes.",
            "match x { Some(token) => token, None => return }",
            "items.map(token => token.trim())",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// Configuration that names the secret in one place and gives it in another: Kubernetes `name`/`value`, XML
    /// `key`/`value` attributes, `ENV NAME value`, `set NAME value`, a `PSK:` line.
    #[test]
    fn configuration_blocks_go() {
        let pw = weak_pw();
        for t in [
            format!("- name: DB_PASSWORD\n  value: {pw}"),
            format!("- name: PASSWORD\n  value: \"{pw}\""),
            format!("name: DB_PASSWORD\nvalue: {pw}"),
            format!("ENV PASSWORD {pw}"),
            format!("<add key=\"password\" value=\"{pw}\"/>"),
            format!("<input type=\"password\" value=\"{pw}\">"),
            format!("<entry key=\"token\">{pw}</entry>"),
            format!("SSID: casa\nPSK: {pw}"),
            format!("aws configure set aws_secret_access_key {pw}"),
        ] {
            assert_gone(&t, &pw);
        }
        for ok in [
            "<input type=\"password\" name=\"password\" placeholder=\"Senha\">",
            "- name: DB_HOST\n  value: localhost",
            "ENV PATH /usr/bin",
            "SELECT id, password_hash FROM users",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// Passwords given to more commands: `docker login -p`, `redis-cli -a`, `zip -P`, `7z -p`, `unrar -p`, `mongo -p`,
    /// `ssh-keygen -N`, `openssl -k` and `-pass pass:`, `az login -p`, `wpa_passphrase`, `vault login`, and piped into
    /// `passwd --stdin`, `chpasswd` or `--password-stdin`.
    #[test]
    fn more_commands_go() {
        let pw = weak_pw();
        for t in [
            format!("docker login -u ana -p {pw}"),
            format!("redis-cli -a {pw}"),
            format!("zip -P {pw} x.zip"),
            format!("7z a -p{pw} x.7z"),
            format!("unrar x -p{pw} a.rar"),
            format!("mongo -p {pw}"),
            format!("ssh-keygen -N {pw}"),
            format!("openssl enc -k {pw}"),
            format!("openssl enc -pass pass:{pw}"),
            format!("az login -p {pw}"),
            format!("wpa_passphrase casa {pw}"),
            format!("nmcli dev wifi connect casa password {pw}"),
            format!("vault login {pw}"),
            format!("echo {pw} | passwd --stdin ana"),
            format!("echo ana:{pw} | chpasswd"),
            format!("echo {pw} | docker login --password-stdin"),
        ] {
            assert_gone(&t, &pw);
        }
        assert_eq!(red(&format!("echo ana:{pw} | chpasswd")), format!("echo ana:{REDACTED} | chpasswd"));
        for ok in
            ["docker run -p 8080:80 nginx", "zip -r x.zip dir", "7z a x.7z dir", "ssh -p 2222 user@host", "openssl enc -pass env:PASS -in a -out b"]
        {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// More JSON keys that hold a secret; `session`, `otp`, `seed`, `dsn` and `bearer` only when their value is text.
    #[test]
    fn more_json_keys_go() {
        let pw = weak_pw();
        let r = Redactor::default();
        for key in ["signing_key", "encryption_key", "master_key", "auth_header", "mnemonic", "session", "sessionid", "otp", "seed", "dsn", "bearer"]
        {
            let v = json!({ key: pw });
            assert_eq!(r.redact_value(&v, &mut Stats::default())[key], REDACTED, "{key}");
            assert_eq!(r.scan_value(&v), 1, "{key}");
        }
        let kept = json!({ "session": { "id": 7 }, "session_count": 3, "seed": 42 });
        assert_eq!(r.redact_value(&kept, &mut Stats::default()), kept);
        assert_eq!(r.scan_value(&kept), 0);
    }

    /// A literal inside code under a secret's name goes (`get_secret("X")`, `or "X"`, `Some("X")`, `== "X"`,
    /// `login(user, "X")`); the names and messages around it stay.
    #[test]
    fn literals_inside_code_go() {
        let pw = weak_pw();
        for t in [
            format!("password = get_secret({pw})"),
            format!("password = get_secret(\"{pw}\")"),
            format!("secret = os.environ.get(\"X\") or \"{pw}\""),
            format!("password = os.getenv('X', '{pw}')"),
            format!("const password = process.env.X || '{pw}';"),
            format!("key = [{pw}]"),
            format!("password = Some(\"{pw}\")"),
            format!("let password = String::from(\"{pw}\");"),
            format!("password == \"{pw}\""),
            format!("if (password === \"{pw}\") {{"),
            format!("assert_eq!(password, \"{pw}\");"),
            format!("password.equals(\"{pw}\")"),
            format!("setPassword(\"{pw}\")"),
            format!("set_password('{pw}')"),
            format!("login(user, \"{pw}\")"),
            format!("$config['password'] = '{pw}';"),
            format!("config[\"password\"] = \"{pw}\""),
            format!("password = await fetch({pw})"),
        ] {
            assert_gone(&t, &pw);
        }
        assert_eq!(red(&format!("secret = os.environ.get(\"X\") or \"{pw}\"")), format!("secret = os.environ.get(\"X\") or \"{REDACTED}\""));
        for ok in [
            "let secret = env::var(\"SECRET_KEY\").expect(\"SECRET_KEY must be set\");",
            "expect(validatePassword(\"correct-horse-battery\")).toBe(true);",
            "const keyId = \"key-2024-11\";",
            "token = create_token_v2(user.id)",
            "password = getpass.getpass(\"Password: \")",
            "secret = config.get(\"secret\")",
            "resp = client.post(\"/token/refresh\", json={\"refresh\": \"stub\"})",
            // (what a trace's own body holds: model names next to token counts, ids in hex)
            "generate(prompt, max_tokens=512, model=\"lfm3-1.7b\")",
            "{\"attributes\": [{\"key\": \"gen_ai.usage.input_tokens\"}, {\"key\": \"gen_ai.request.model\", \"value\": \"lfm3-1.7b\"}]}",
            "let token = Token::new(span, \"0af7651916cd43dd8448eb211c80319c\");",
            "{\"password_hint_shown\": true, \"model\": \"lfm3-1.7b\", \"x\": [1]}",
            "const password = document.querySelector('#password').value;",
            "password = \"<your password>\"",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// The critic's ordinary texts that were cut (round 3): prose about a password, a placeholder, a header example,
    /// code that copies a variable, SQL placeholders, names that only name a secret, JSON about a token's type.
    #[test]
    fn the_critics_ordinary_texts_stay() {
        for ok in [
            "Veja a seção de segurança para saber como guardamos a senha do usuário: usamos Argon2id com salt de 16 bytes.",
            "`POST /login` recebe `{\"username\": \"...\", \"password\": \"...\"}` e devolve `{\"token\": \"...\"}`.",
            "The `Authorization` header must carry a bearer token. Example: `Authorization: Bearer <token>`.\nNever log the token or the password.",
            "pub fn set_password(&mut self, password: String) {\n    self.password = password;\n}",
            "En el formulario, el campo contraseña es obligatorio y debe tener al menos ocho caracteres.",
            "パスワードは8文字以上で入力してください。",
            "const secret = \"jwt-secret-name\";\nconst keyId = \"key-2024-11\";",
            "- Corrigir bug no login: a senha com acento não era aceita",
            "export PASSWORD_MIN_LENGTH=12\nexport TOKEN_TTL=3600\nexport SECRET_BACKEND=vault",
            "auth:\n  method: password\n  token_ttl: 3600\n  secret_ref: vault://kalango/db",
            "const API_KEY_HEADER = \"X-API-Key\";\nconst TOKEN_ENDPOINT = \"https://auth.example.com/oauth/token\";",
            "UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1;",
            "{\n  \"status\": \"ok\",\n  \"token_type\": \"bearer\",\n  \"expires_in\": 3600\n}",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// JSON written as text keeps its layout when something in it is redacted: only the value changes.
    #[test]
    fn redacted_json_text_keeps_its_layout() {
        let pw = weak_pw();
        let text = format!("{{\n  \"user\": \"ana\",\n  \"password\": \"{pw}\",\n  \"n\": [1, 2]\n}}");
        assert_eq!(red(&text), format!("{{\n  \"user\": \"ana\",\n  \"password\": \"{REDACTED}\",\n  \"n\": [1, 2]\n}}"));
        let env = "{\n \"z\": 1,\n \"env\": {\"A\": \"1\"},\n \"a\": true\n}";
        assert_eq!(red(env), "{\n \"z\": 1,\n \"env\": \"[REDACTED:env]\",\n \"a\": true\n}");
    }

    /// The final scan descends into the values of objects and arrays by itself (not only through the text of the body).
    #[test]
    fn scan_value_at_descends_into_values() {
        let r = Redactor::default();
        assert_eq!(r.scan_value_at(&json!({ "outer": { "password": "v" } }), 0), 1);
        assert_eq!(r.scan_value_at(&json!([{ "token": "v" }, { "ok": 1 }]), 0), 1);
        assert_eq!(r.scan_value_at(&json!({ "a": [{ "b": { "secret": "v" } }] }), 0), 1);
        assert_eq!(r.scan_value_at(&json!({ "outer": { "password": REDACTED } }), 0), 0);
    }

    /// Less common shapes of the same secrets: a name that describes a secret before a credential, `?=`, a bare
    /// comparison, a YAML tag, a heading or a table cell, `senha - X`, URL parameters, generic names (`key`, `chave`)
    /// before a credential, a token as a URL's user, Japanese quotes.
    #[test]
    fn less_common_shapes_go() {
        let pw = weak_pw();
        for t in [
            format!("password_policy: {pw}"),
            format!("token_ttl = {pw}"),
            format!("password ?= {pw}"),
            format!("password == {pw}"),
            format!("password: !!str {pw}"),
            format!("password: !vault {pw}"),
            format!("# Senha\n\n{pw}"),
            format!("Senha\n-----\n{pw}"),
            format!("Senha:\n\n    {pw}"),
            format!("| senha | {pw} |"),
            format!("senha - {pw}"),
            format!("senha → {pw}"),
            format!("https://x/y?sig={pw}&a=1"),
            format!("https://x/y?key={pw}"),
            format!("chave: {pw}"),
            format!("use this key: {pw}"),
            format!("https://{pw}@x/"),
            format!("私のパスワードは「{pw}」です"),
        ] {
            assert_gone(&t, &pw);
        }
        for ok in [
            "key: value",
            "password_policy: strict",
            "token_ttl = 3600",
            "secret: !vault |",
            "| password | string | senha do usuário |",
            "git clone https://git@github.com/x/y",
            "https://x/y?key=abc&code=xyz",
            "a chave do carro ficou na mesa",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// A GitHub Actions expression, `${{ … }}`, is a whole reference: it stays, and nothing of it is cut halfway; one
    /// that holds a credential instead of a name goes whole.
    #[test]
    fn github_actions_expressions_stay_whole() {
        for ok in [
            "TOKEN: ${{ secrets.GITHUB_TOKEN }}",
            "password: ${{ secrets.DB_PASSWORD }}",
            "GH_TOKEN: ${{ github.token }}",
            "env:\n  API_KEY: ${{ secrets.API_KEY }}\n  X: 1",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
        // (a reference only with an expression inside: a bare credential is not one)
        let pw = weak_pw();
        assert_gone(&format!("password: ${{{{ {pw} }}}}"), &pw);
    }

    /// A password prompt answered on its line (`Password for ana@host: X`, `[sudo] password for ana: X`): the answer
    /// goes, whole, and who it is for stays.
    #[test]
    fn a_password_prompt_with_its_answer_goes() {
        let pw = concat!("Tr0ub", "4dor&3");
        assert_eq!(red(&format!("Password for ana@host: {pw}")), format!("Password for ana@host: {REDACTED}"));
        assert_eq!(red(&format!("[sudo] password for ana: {pw}")), format!("[sudo] password for ana: {REDACTED}"));
        assert_gone(&format!("Enter password for root: {pw}"), "4dor");
        assert_eq!(red("Password for ana@host:"), "Password for ana@host:");
        // (the answer goes whatever its shape)
        assert_eq!(red("Password for ana@host: correcthorse"), format!("Password for ana@host: {REDACTED}"));
    }

    /// Names that are about a password, not the password: a timestamp under `passwordChangedAt`, an i18n key
    /// (`settings.password.title`), a boarding pass, a flag talked about in prose, `--password-stdin`.
    #[test]
    fn names_about_a_password_stay() {
        for ok in [
            "passwordChangedAt: 2026-11-26",
            "{\"passwordChangedAt\": \"2026-11-26T10:00:00Z\"}",
            "\"settings.password.title\": \"Senha\"",
            "settings.password.placeholder = Digite sua senha",
            "Boarding pass: seat 14C",
            "Use the --token flag to pass the key.",
            "docker login --password-stdin registry.example.com",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
        // (the same names before a credential still lose it)
        let pw = weak_pw();
        let r = Redactor::default();
        assert_eq!(r.redact_value(&json!({ "passwordChangedAt": pw }), &mut Stats::default())["passwordChangedAt"], REDACTED);
        assert_gone(&format!("settings.password.title = {pw}"), &pw);
        assert_gone(&format!("tool --token {pw}"), &pw);
    }

    /// The final scan on its own, piece by piece: an environment key that kept its value, a key that is a token, and a
    /// secret that only decoding the JSON inside a string shows.
    #[test]
    fn the_final_scan_reads_env_keys_object_keys_and_json_in_strings() {
        let r = Redactor::default();
        assert_eq!(r.scan_value_at(&json!({ "env": { "HOME": "/home/ana" } }), 0), 1, "an environment key with its value");
        assert_eq!(r.scan_value_at(&json!({ "env": "[REDACTED:env]" }), 0), 0);
        let token = concat!("ghp", "_abcdefghijklmnopqrstuvwxyz0123456789");
        assert!(r.scan_value_at(&json!({ token: 1 }), 0) >= 1, "a key that is a token");
        let inner = format!("{{\"pass\\u0077ord\": \"{}\"}}", weak_pw());
        assert!(r.scan_plain_text(&inner) == 0, "the text alone does not show it");
        assert_eq!(r.scan_value_at(&json!({ "note": inner }), 0), 1, "decoding the JSON in the string does");
    }

    /// A value is a date only in a real layout (`YYYY-MM-DD`, then `THH:MM:SS`, a fraction and a zone), and only under a
    /// key about time (`passwordChangedAt`, `token_updated_at`): a number with dashes or a `T` and a `Z` in it is not a
    /// date, and under any other key a date-looking value is judged as a value.
    #[test]
    fn only_real_dates_under_time_keys_stay() {
        let r = Redactor::default();
        for (key, value) in [
            ("passwordChangedAt", "2024-99-99"),
            ("passwordChangedAt", "1234-5678-9012"),
            ("token_updated_at", "2026-11-26T25:00:00Z"),
            ("password_policy", "2026-11-26T10:00:00Z"),
            ("secret_name", "88123311T9Z"),
        ] {
            let v = json!({ key: value });
            assert_eq!(r.redact_value(&v, &mut Stats::default())[key], REDACTED, "{key}: {value}");
            assert_eq!(r.scan_value(&v), 1, "{key}: {value}");
        }
        let code = concat!("8812-3", "311T9Z");
        assert_gone(&format!("https://x/y?code={code}&a=1"), code);
        assert_gone(&format!("password_policy: {code}"), code);
        // (a real date under a name that is not about time is judged as a value)
        assert_gone("password_policy: 2026-11-26T10:00:00Z", "10:00:00");
        for ok in ["passwordChangedAt: 2026-11-26", "token_updated_at: 2026-11-26T10:00:00.123+03:00", "{\"password_changed\": \"2026-11-26 10:00\"}"]
        {
            assert_eq!(red(ok), ok, "{ok}");
        }
        assert!(timestamp("2026-11-26T10:00:00Z") && timestamp("2026-02-31") && !timestamp("2026-13-01") && !timestamp("2026-11-26T24:00"));
    }

    /// A word ending in `:` after a password's name is the value when nothing follows it on its line (`root password
    /// X:`), and a "who:" when something does (`root password admin: X`): the value goes, without its colon.
    #[test]
    fn a_value_before_a_colon_still_goes() {
        let pw = weak_pw();
        assert_eq!(red(&format!("root password {pw}:")), format!("root password {REDACTED}:"));
        assert_eq!(red(&format!("senha atual {pw}:\nfim")), format!("senha atual {REDACTED}:\nfim"));
        assert_eq!(red(&format!("root password admin: {pw}")), format!("root password admin: {REDACTED}"));
        assert_gone(&format!("pass ana@host: {pw}"), &pw);
    }

    /// The inputs of the performance test, `n` bytes each (cut on a character): every group of detectors, minified JSON
    /// (no spaces: a look-behind that runs to the previous space runs to the start), and `{"a":[{"password":"` repeated.
    fn perf_inputs(n: usize) -> Vec<(&'static str, String)> {
        let fill = |unit: &str| {
            let mut s = unit.repeat(n / unit.len() + 1);
            let mut cut = n;
            while !s.is_char_boundary(cut) {
                cut -= 1;
            }
            s.truncate(cut);
            s
        };
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let random: String = (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (b' ' + (x % 95) as u8) as char
            })
            .collect();
        vec![
            ("pw=", fill("password=hunter2 api_key=abc123 ")),
            ("prose", fill("a senha é bolo de cenoura e minha senha do wifi é Kalango2024. my password is correct horse. ")),
            ("braces", fill("TOKEN: ${{ secrets.X }} pw=x1 {{ y }} ${A} $(cat f) ")),
            ("short pairs", fill("pw=x1 ")),
            // (`pass` names with no space between them: a look-behind for each runs to the start without a bound)
            ("bare pass", fill("pass=x1pass=")),
            ("markup", format!("{}</x>", fill("<password>abc"))),
            ("flags", fill("mysql -p x --password y docker login -p z | sudo -S curl -u a:b ")),
            ("random", random),
            ("minified JSON", fill("{\"name\":\"ana\",\"password\":\"x1\",\"token\":\"y2\"},")),
            ("{\"a\":[{\"password\":\"", fill("{\"a\":[{\"password\":\"")),
            // (`pass` after a quote, an `&` or a `;` with a short value: the value ends before the next name, and the
            // look-behind of every name could run to the start)
            ("{\"pass\":\"x\"},", fill("{\"pass\":\"x\"},")),
            ("\"pass\":1,", fill("\"pass\":1,")),
            ("&pass=a&", fill("&pass=a&")),
            (";pass=a;", fill(";pass=a;")),
        ]
    }

    /// The detectors take time in proportion to the text (1 MB is about twice 500 KB; four times would be quadratic), with no input that makes one of them
    /// look back or ahead to the start or the end of the text at every step. `redact_str` is capped at 64 KiB, so the
    /// detectors are driven directly; each run has a generous bound for a debug build, and stops the test at once when
    /// it is exceeded.
    /// (About a minute of a debug build at full load, which starves the tests with deadlines that run beside it: it is
    /// left out of the default run; run it with `cargo test -- --ignored the_detectors_are_linear_on_one_megabyte`.)
    #[test]
    #[ignore]
    fn the_detectors_are_linear_on_one_megabyte() {
        const BOUND: std::time::Duration = std::time::Duration::from_secs(60);
        let timed = |text: String| {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let r = Redactor::default();
                let t = std::time::Instant::now();
                let _ = r.text_pipeline(&text, &mut Stats::default());
                let _ = r.scan_text(&text);
                let _ = tx.send(t.elapsed());
            });
            rx.recv_timeout(BOUND).ok()
        };
        // (the best of two runs: the machine is shared, and a busy moment only makes a run slower)
        let best = |text: &str, what: &str| {
            (0..2).map(|_| timed(text.to_string()).unwrap_or_else(|| panic!("{what} took more than {BOUND:?}"))).min().unwrap()
        };
        let half = perf_inputs(500 * 1024);
        let full = perf_inputs(1024 * 1024);
        for ((name, h), (_, f)) in half.into_iter().zip(full) {
            let th = best(&h, &format!("{name}: 500 KB"));
            let tf = best(&f, &format!("{name}: 1 MB"));
            eprintln!("PERF {name:<22} 500 KB {th:>12.3?}  1 MB {tf:>12.3?}  ratio {:.2}", tf.as_secs_f64() / th.as_secs_f64());
            // (below a few milliseconds the ratio is noise)
            if th.as_millis() >= 20 {
                assert!(tf.as_secs_f64() < 3.5 * th.as_secs_f64(), "{name}: 1 MB took {tf:?}, 500 KB {th:?}: not linear");
            }
            // (a release build: 1 MB of any input in under three seconds)
            assert!(cfg!(debug_assertions) || tf < std::time::Duration::from_secs(3), "{name}: 1 MB took {tf:?}");
        }
    }

    /// Judging a value reads a bounded window after it: the same value before 8 MB of text without a line break costs
    /// what it costs before a few bytes (a look-ahead to the end of the line made every pair of a long line quadratic).
    #[test]
    fn judging_a_value_reads_a_bounded_window() {
        let r = Redactor::default();
        let long = format!(" {}", "x".repeat(8 << 20));
        let time = |rest: &str| {
            let t = std::time::Instant::now();
            for _ in 0..3000 {
                std::hint::black_box(r.value_shaped(std::hint::black_box("hunter2"), rest, true, "password"));
            }
            t.elapsed()
        };
        let (short, far) = (time(" ok"), time(&long));
        assert!(far < short * 50 + std::time::Duration::from_millis(200), "{far:?} before 8 MB, {short:?} before a few bytes");
    }

    /// `${{ … }}` is one unit everywhere: after a flag, a scheme, in a URL, piped, glued to a word, with braces of its
    /// own; a secret glued to it still goes, and a JSON value or an environment made only of it stays.
    #[test]
    fn github_actions_expressions_are_one_unit_everywhere() {
        for ok in [
            "run: docker login -u ana -p ${{ secrets.PW }} ghcr.io",
            concat!("run: cu", "rl -H \"Authorization: Bearer ${{ secrets.API }}\" https://x"),
            "run: mysql -u root -p${{ secrets.DB }} -h x",
            // (the command name put together: a secret scanner reads it with `-u user:x` as a credential)
            concat!("run: cu", "rl -u ana:${{ secrets.PW }} https://x"),
            "run: echo \"${{ secrets.TOKEN }}\" | docker login -u ana --password-stdin ghcr.io",
            "password: ${{ format('{0}', secrets.X) }}",
            "token: ${{ matrix.os }}-latest",
            "password: ${{ secrets.X",
            "{\"steps\":[{\"with\":{\"token\":\"${{ secrets.PAT }}\"}}]}",
            "{\"env\":{\"TOKEN\":\"${{ secrets.GITHUB_TOKEN }}\"}}",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
        let pw = weak_pw();
        assert_eq!(red(&format!("password: ${{{{ secrets.A }}}}{pw}")), format!("password: ${{{{ secrets.A }}}}{REDACTED}"));
        assert_eq!(red(&format!("TOKEN: ${{{{ secrets.A }}}} PASSWORD: {pw}")), format!("TOKEN: ${{{{ secrets.A }}}} PASSWORD: {REDACTED}"));
        let json = format!("{{\"token\":\"${{{{ secrets.A }}}}\",\"password\":\"{pw}\"}}");
        assert_eq!(red(&json), format!("{{\"token\":\"${{{{ secrets.A }}}}\",\"password\":\"{REDACTED}\"}}"));
        let r = Redactor::default();
        let formatted = json!({ "token": "${{ format('{0}', secrets.X) }}" });
        assert_eq!(r.redact_value(&formatted, &mut Stats::default()), formatted);
        assert_eq!(r.scan_value(&json!({ "env": { "TOKEN": "${{ secrets.X }}" } })), 0);
        assert_eq!(r.scan_value(&json!({ "env": { "TOKEN": "${{ secrets.X }}", "HOME": "/home/ana" } })), 1);
    }

    /// Ordinary texts that were cut: a token read from a file after `Bearer`, a token's type before a count on the next
    /// line, a model's name, a reference as a URL's password, a template's field path, prose after a label, a program's
    /// message, PowerShell, a flag between backticks, the label of a field.
    #[test]
    fn more_ordinary_texts_stay() {
        for ok in [
            "Authorization: Bearer $(cat ~/.cache/token)",
            "token_type: bearer\nexpires_in: 3600",
            "tokenizer = AutoTokenizer.from_pretrained(\"LiquidAI/LFM3-1.7B\")",
            "tok = load_tokenizer(\"lfm3-1.7b\")",
            "DATABASE_URL=postgres://app:${DB_PASSWORD}@db:5432/app",
            "redis://:${REDIS_PASSWORD}@cache:6379/0",
            "data:\n  password: {{ .Values.db.passwordB64 }}",
            "A senha do Wi-Fi fica na etiqueta embaixo do roteador.",
            "chpasswd: password for 'ana' changed",
            "passwd: password updated successfully",
            "$password = Read-Host -AsSecureString \"Senha\"",
            "$env:API_TOKEN = $env:CI_JOB_TOKEN",
            "Run `app --password-stdin` to read the password from standard input.",
            "label_password = \"Senha\"\nplaceholder_token = \"Cole o token aqui\"",
            "\"settings.token.hint\": \"O token expira em 24 horas\"",
            "Auth scheme: Bearer\nsession2024 started",
            "token = read(\"secrets/token2024.txt\")",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
        // (and the same places with a credential still lose it)
        let pw = weak_pw();
        for t in [
            format!("Authorization: Bearer $(echo {pw})"),
            format!("postgres://app:{pw}@db/app"),
            format!("label_password = \"{pw}\""),
            format!("passwd: {pw}"),
            format!("tokenizer = load(\"{pw}\", password=\"{pw}\")"),
        ] {
            assert_gone(&t, &pw);
        }
    }

    /// The checks the critic's mutants showed no test needed: `*_date` is a key about time, a quoted scheme word
    /// alone is not an interpolation, a credential-shaped text with a secret's word is a credential, a placeholder holds
    /// words, and a zone ends a date.
    #[test]
    fn the_rules_the_mutants_found_untested() {
        let r = Redactor::default();
        let kept = json!({ "password_date": "2026-11-26" });
        assert_eq!(r.redact_value(&kept, &mut Stats::default()), kept);
        assert_gone("password: \"basic\"", "basic");
        assert_gone(&format!("secret: \"{}\"", concat!("api-key-", "2024x9")), "2024x9");
        assert_gone(&format!("password: \"<{}>\"", weak_pw()), &weak_pw());
        assert!(!timestamp("2026-11-26T10:00:00Zbanana"));
    }

    /// `${{ … }}` is a reference only when it holds an expression (names, fields, strings and known functions): one that
    /// holds a pair or a credential is text, and the detectors read it.
    #[test]
    fn an_expression_holding_a_secret_is_not_a_reference() {
        let pw = weak_pw();
        for t in [format!("run: echo ${{{{ password={pw} }}}}"), format!("TOKEN: ${{{{ '{pw}' }}}}"), format!("password: ${{{{ {pw} }}}}")] {
            assert_gone(&t, &pw);
        }
        assert_gone("TOKEN: ${{ secrets.A = secrets.B }}", "secrets.A");
        assert_gone(&format!("token: ${{{{ github.actor == '{pw}' }}}}"), &pw);
        let r = Redactor::default();
        let v = json!({ "password": format!("${{{{ '{pw}' }}}}") });
        assert_eq!(r.redact_value(&v, &mut Stats::default())["password"], REDACTED);
        let v = json!({ "token": format!("${{{{ '{pw}' }}}}") });
        assert_eq!(r.redact_value(&v, &mut Stats::default())["token"], REDACTED, "under any secret's name");
        for ok in [
            "TOKEN: ${{ secrets.GITHUB_TOKEN }}",
            "password: ${{ format('{0}', secrets.X) }}",
            "if: ${{ github.event_name == 'push' && matrix.os != 'windows' }}",
            "token: ${{ matrix.os }}-latest",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// A PowerShell cmdlet is code only when it is a known one or is given a parameter, and never under a password's
    /// name: `password: Hunter-Two` is a password.
    #[test]
    fn a_cmdlet_is_code_only_when_it_is_one() {
        for t in ["password: Hunter-Two", "password = Hunter-Two", "password: Hunter-Two -Force", "token = Blue-Sky"] {
            assert_gone(t, "Two");
        }
        assert_gone("token = Blue-Sky", "Sky");
        for ok in ["$password = Read-Host -AsSecureString \"Senha\"", "$token = Get-Content token.txt", "$cred = Get-Credential"] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// A literal with `/` in code is a path or a model's name only in those shapes; a base64-looking literal with `/`
    /// is a credential.
    #[test]
    fn a_slash_does_not_make_a_literal_a_path() {
        let b64 = concat!("Zk9vYmFy", "/QmF6cXV4", "+MTIzNDU2Nzg5MA==");
        assert_gone(&format!("token = decode(\"{b64}\")"), "QmF6cXV4");
        assert_gone(&format!("password = get_secret(\"{}\")", concat!("a1b2", "/c3d4")), "c3d4");
        for ok in [
            "tokenizer = AutoTokenizer.from_pretrained(\"LiquidAI/LFM3-1.7B\")",
            "password = load_model(\"LiquidAI/LFM3-1.7B\")",
            "token = read(\"secrets/token2024.txt\")",
            "token = open(\"./config/v2/token.json\")",
            "token = open(\"~/.config/kalango/token2\")",
            "token = read(\"config/keys/token2024.json\")",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// A quoted sentence under a name that is not a password's is prose only with the marks of prose (a short word of
    /// a language, or punctuation); a passphrase of plain words under any secret's name goes.
    #[test]
    fn a_quoted_passphrase_is_not_prose() {
        let words = concat!("correct horse ", "battery staple");
        for t in [format!("password=\"{words}\""), format!("secret = \"{words}\""), format!("api_key: \"{words}\"")] {
            assert_gone(&t, "battery");
        }
        for ok in [
            "\"settings.token.hint\": \"O token expira em 24 horas\"",
            "placeholder_token = \"Cole o token aqui\"",
            "token_help = \"Paste the token here.\"",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// A quoted string inside `${{ … }}` is an operand of a comparison, never the whole expression nor a fallback
    /// (`|| 'x'`); after a password's name, not even an operand that looks like a credential, a number or a phrase.
    #[test]
    fn a_string_in_an_expression_is_only_compared() {
        let pw = weak_pw();
        for (t, secret) in [
            (format!("password: ${{{{ '{pw}' }}}}"), pw.clone()),
            (format!("password: ${{{{ secrets.X || '{}' }}}}", concat!("hun", "ter")), concat!("hun", "ter").to_string()),
            (format!("token: ${{{{ secrets.X || '{pw}' }}}}"), pw.clone()),
            (format!("PIN: ${{{{ '{}' }}}}", concat!("4829", "13")), concat!("4829", "13").to_string()),
            (format!("password: ${{{{ github.actor == '{}' }}}}", concat!("4829", "13")), concat!("4829", "13").to_string()),
        ] {
            assert_gone(&t, &secret);
        }
        for ok in [
            "TOKEN: ${{ secrets.GITHUB_TOKEN }}",
            "os: ${{ matrix.os }}",
            "if: ${{ github.event_name == 'push' }}",
            "if: ${{ matrix.os == 'ubuntu-latest' && github.ref != 'refs/heads/main' }}",
            "password: ${{ format('{0}', secrets.X) }}",
            "password: ${{ secrets.DB_PASSWORD }}",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// The strict check of an expression after a password's or PIN's name holds whatever stands between the name and
    /// `${{`: quotes, spaces, a JSON key, a YAML block on the next line, and a JSON value under such a key.
    #[test]
    fn the_strict_check_follows_the_name_through_quotes_and_lines() {
        let code = concat!("4829", "13");
        for t in [
            format!("password: \"${{{{ github.actor == '{code}' }}}}\""),
            format!("password='${{{{ github.actor == '{code}' }}}}'"),
            format!("password = ${{{{ github.actor == '{code}' }}}}"),
            format!("\"password\": \"${{{{ github.actor == '{code}' }}}}\""),
            format!("password: |\n  ${{{{ github.actor == '{code}' }}}}"),
            format!("PIN: >\n    ${{{{ github.actor == '{code}' }}}}"),
            format!("my password is \"${{{{ github.actor == '{code}' }}}}\""),
        ] {
            assert_gone(&t, code);
        }
        let r = Redactor::default();
        let v = json!({ "password": format!("${{{{ github.actor == '{code}' }}}}") });
        assert_eq!(r.redact_value(&v, &mut Stats::default())["password"], REDACTED);
        for ok in [
            "password: \"${{ secrets.DB_PASSWORD }}\"",
            "\"password\": \"${{ secrets.DB_PASSWORD }}\"",
            "password: |\n  ${{ secrets.DB_PASSWORD }}",
            "token: \"${{ github.event_name == 'push' }}\"",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
        let env = json!({ "env": { "PASSWORD": format!("${{{{ github.actor == '{code}' }}}}") } });
        assert_eq!(r.redact_value(&env, &mut Stats::default())["env"], "[REDACTED:env]");
        let kept = json!({ "password": "${{ secrets.DB_PASSWORD }}" });
        assert_eq!(r.redact_value(&kept, &mut Stats::default()), kept);
    }

    /// A `${{ … }}` that is not an expression goes whole, wherever it is, when it holds a credential or follows a
    /// password's or a PIN's name: after a flag, `Bearer`, in a URL, glued to the flag or to another block, a fallback,
    /// a bare number; nothing of the literal is left.
    #[test]
    fn a_block_holding_a_secret_goes_whole_anywhere() {
        let x = concat!("banana", "2024xyz");
        for t in [
            format!("--password ${{{{ '{x}' }}}}"),
            format!("docker login -u ana -p ${{{{ '{x}' }}}}"),
            format!("postgres://app:${{{{ '{x}' }}}}@db/x"),
            format!("Bearer ${{{{ '{x}' }}}}"),
            format!("--password ${{{{ secrets.X || '{x}' }}}}"),
            format!("echo ${{{{ '{x}' }}}} | sudo -S ls"),
            format!("token: ${{{{ format('{{0}}', '{x}') }}}}"),
            format!("password: ${{{{ 'a' }}}}${{{{ '{x}' }}}}"),
            format!("password: ${{{{ github.event.inputs.pw }}}}${{{{ '{x}' }}}}"),
        ] {
            assert_gone(&t, x);
        }
        for (t, gone) in [
            (format!("mysql -p${{{{ '{x}' }}}}"), "mysql -p[REDACTED]"),
            (format!("{0} -u ana:${{{{ '{x}' }}}} https://h", concat!("cu", "rl")), concat!("cu", "rl -u ana:[REDACTED] https://h")),
        ] {
            assert_eq!(red(&t), gone, "{t}");
        }
        for t in ["pin: ${{ 482913 }}", "password: ${{ 12345678 }}"] {
            assert_gone(t, &t[t.find("{{ ").unwrap() + 3..t.find(" }}").unwrap()]);
        }
        // (only a credential's shape makes a word go: `Bearer ${{ X }}`, `${{ secrets.X || 'p@ss!word' }}`; and after a
        // password's name in a sentence, the whole block)
        assert_gone(&format!("Bearer ${{{{ {x} }}}}"), x);
        assert_gone("Bearer ${{ secrets.X || 'p@ss!word' }}", "ss!word");
        assert_gone("my password is ${{ 12345678 }}", "12345678");
        // (an invalid block that holds no credential is still not a reference under a secret's name)
        assert_eq!(red("token: ${{ inputs.x || 'world' }}"), format!("token: {REDACTED}"));
        assert_eq!(red("token='${{ inputs.x || 'world' }}'"), format!("token='{REDACTED}'"));
        let r = Redactor::default();
        assert_eq!(r.redact_value(&json!({ "token": "${{ inputs.x || 'world' }}" }), &mut Stats::default())["token"], REDACTED);
        assert_gone(&format!("password: |\n  {x}"), x);
        for ok in
            ["cache-key: ${{ runner.os }}-${{ hashFiles('**/package-lock.json') }}", "timeout: ${{ 30 }}", "name: ${{ inputs.name || 'world' }}"]
        {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// Hex after a key's label is a key (`api key: 8c1d…`, `chave da API: 9f8e…`); outside a label, a hash or an id.
    #[test]
    fn hex_after_a_key_label_is_a_key() {
        let hex = concat!("8c1d4e7f", "2a9b3c6d5e0f1a2b");
        for t in [
            format!("api key: {hex}"),
            format!("API key: {hex}"),
            format!("secret key: {hex}"),
            format!("chave: {hex}"),
            format!("chave da API: {}", &hex[..16]),
        ] {
            assert_gone(&t, &hex[..16]);
        }
        for ok in [
            "commit 6219cf1a2b3c4d5e6f708192a3b4c5d6e7f80912",
            "sha256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "id: 550e8400-e29b-41d4-a716-446655440000",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// The round-6 critic's ordinary texts: a namespace or a lifetime under a secret's name, a method in JSON, a
    /// password given to curl as a variable.
    #[test]
    fn the_round_six_ordinary_texts_stay() {
        for ok in [
            "secret_name: db-credentials\nsecret_namespace: production",
            "refresh_token_rotation = true\naccess_token_lifetime = \"15m\"",
            "{\"token_endpoint_auth_method\": \"client_secret_basic\"}",
            "curl -u \"$USER:$PASSWORD\" https://example.com/private",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
        let r = Redactor::default();
        assert_eq!(r.redact_value(&json!({ "passwordChangedAt": "2024-99-99" }), &mut Stats::default())["passwordChangedAt"], REDACTED);
    }

    /// The final scan reads a text with its backslash escapes as separators: a pair after a written `\n`, which the
    /// first pass reads as one word, is seen.
    #[test]
    fn the_final_scan_reads_escaped_line_breaks() {
        let r = Redactor::default();
        let text = format!("user=ana\\npass={}", concat!("banana", "2024xyz"));
        assert!(r.scan_plain_text(&text) > 0, "{text}");
    }

    /// After `:` under a password's name, `none`, `true` and `false` are the password; a language's `None`/`True` after
    /// `=` is a value of the code. `curl -u user:X` spares only a shell variable.
    #[test]
    fn a_word_is_a_password_where_a_password_goes() {
        for t in ["password: none", "password: true", "senha: false"] {
            assert_gone(t, t.rsplit(' ').next().unwrap());
        }
        // (the command name put together: a secret scanner reads it with `-u user:x` as a credential)
        let client = concat!("cu", "rl");
        for w in ["none", "true", "await"] {
            assert_eq!(red(&format!("{client} -u ana:{w} https://x")), format!("{client} -u ana:{REDACTED} https://x"), "{w}");
        }
        for ok in [
            "password = None",
            "password_require_symbol: true",
            "{\"password\": true, \"x\": 1}",
            "password_reset_email_sent = True",
            "token: none",
            "curl -u \"$USER:$PASSWORD\" https://x",
            "curl -u ana:${DB_PASS} https://x",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// A key that describes a secret keeps a value only when the value describes: a lifetime a duration, a namespace a
    /// DNS name, other descriptions neither credential-shaped nor a secret literal nor with a digit.
    #[test]
    fn a_description_keeps_only_a_description() {
        let r = Redactor::default();
        for (k, v) in [
            ("access_token_lifetime", "hunter-two"),
            ("secret_namespace", "Prod_Secret"),
            ("secret_policy", "Hunter-Two!"),
            ("api_key_namespace", "Zq8kLm2pQ7"),
            ("secret_namespace", "Hunter Two"),
            ("secret_policy", "HunterTwo"),
            ("secret_policy", "a1"),
        ] {
            assert_eq!(r.redact_value(&json!({ k: v }), &mut Stats::default())[k], REDACTED, "{k}: {v}");
        }
        assert_gone("access_token_lifetime: hunter-two", "hunter-two");
        for ok in [
            "access_token_lifetime = \"15m\"",
            "secret_namespace: production",
            "{\"access_token_lifetime\": \"PT15M\"}",
            "{\"token_endpoint_auth_method\": \"client_secret_basic\"}",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// The permissions of a lifetime and a namespace come on top of the other checks: a namespace that is a key in hex,
    /// a secret literal or a mix of letters and digits, or a lifetime that is no duration, goes; a duration stays.
    #[test]
    fn a_lifetime_or_a_namespace_is_still_checked() {
        let hex = concat!("a1b2c3d4e5f6", "a7b8c9d0e1f2");
        let mixed = concat!("Zq8kLm2", "pQ7xV");
        for (t, secret) in [
            (format!("api_key_namespace: {hex}"), hex.to_string()),
            (format!("token_namespace: {mixed}"), mixed.to_string()),
            (format!("secret_lifetime: {}", &mixed[..11]), mixed[..11].to_string()),
            (format!("token_namespace: {}", concat!("ab12", "cd34ef")), concat!("ab12", "cd34ef").to_string()),
            (format!("token_namespace: {}", concat!("zq8-km2", "-pq7")), concat!("zq8-km2", "-pq7").to_string()),
        ] {
            assert_gone(&t, &secret);
        }
        let r = Redactor::default();
        for (k, v) in [
            ("api_key_namespace", hex),
            ("token_namespace", concat!("ab12", "cd34ef")),
            ("token_namespace", concat!("zq8-km2", "-pq7")),
            ("secret_lifetime", &mixed[..11]),
            ("secret_lifetime", concat!("zq8", "km2pq7")),
        ] {
            assert_eq!(r.redact_value(&json!({ k: v }), &mut Stats::default())[k], REDACTED, "{k}: {v}");
        }
        for ok in [
            "access_token_lifetime = \"15m\"",
            "secret_namespace: production",
            "{\"secret_namespace\": \"team-payments\"}",
            "token_lifetime: 1500ms",
            "refresh_token_lifetime = \"90min\"",
            "{\"token_lifetime\": \"3600sec\"}",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// The look-behind of a `pass` name reads a bounded window: after 8 MB of text without a space it costs what it
    /// costs after a few bytes (a look-behind to the previous space made every `pass` of JSON without spaces, of a
    /// query string or of a cookie quadratic).
    #[test]
    fn the_pass_look_behind_reads_a_bounded_window() {
        let long = "x".repeat(8 << 20);
        let time = |s: &str| {
            let t = std::time::Instant::now();
            for _ in 0..200 {
                std::hint::black_box(pass_ticket(std::hint::black_box(s), s.len()));
            }
            t.elapsed()
        };
        let (short, far) = (time("x"), time(&long));
        assert!(far < short * 50 + std::time::Duration::from_millis(200), "{far:?} after 8 MB, {short:?} after a few bytes");
        let ticket = format!("{} Boarding ", "x".repeat(100));
        assert!(pass_ticket(&ticket, ticket.len()) && pass_ticket("Boarding", 8) && !pass_ticket(&long, long.len()));
    }

    /// The rules the critic's mutants found untested: a shell variable is a whole name (`$PASSWORD-literal` is a word
    /// glued to one), a literal shaped like a credential in an expression after a password's name is not a reference,
    /// and sixteen hex letters with no digit (`deadbeefdeadbeef`) are a placeholder, not a key.
    #[test]
    fn the_round_seven_rules_are_tested() {
        for (t, secret) in [
            (concat!("cu", "rl -u ana:$PASSWORD-literal https://x"), "$PASSWORD-literal"),
            (concat!("cu", "rl -u ana:$A.b https://x"), "$A.b"),
            ("password: ${{ github.ref == 'HunterTwo' }}", "HunterTwo"),
            ("pin: ${{ github.ref == 'HunterTwo' }}", "HunterTwo"),
            // (only a `pass` is a ticket: "season password: X" is a password)
            (concat!("season password: bana", "na2024"), concat!("bana", "na2024")),
        ] {
            assert_gone(t, secret);
        }
        for ok in ["key: deadbeefdeadbeef", "signing key: fadedcafebeadbad", concat!("cu", "rl -u ana:$PASSWORD https://x")] {
            assert_eq!(red(ok), ok, "{ok}");
        }
    }

    /// An object or a list under a name that describes a secret goes whole (fail closed): a field of a neutral name in
    /// it, `{"password_policy": {"default": X}}`, may hold the secret, in a value and in text; a scalar about the
    /// secret, `password_length: 12`, stays.
    #[test]
    fn an_object_under_a_secret_name_goes_whole() {
        let r = Redactor::default();
        let pw = weak_pw();
        let key = concat!("Zq8kLm2", "pQ7x");
        for (k, v, secret) in [
            ("password_policy", json!({"default": pw}), pw.as_str()),
            ("password_policy", json!({"value": pw}), pw.as_str()),
            ("token_scopes", json!(["read", pw]), pw.as_str()),
            ("password_policy", json!([pw]), pw.as_str()),
            ("password_policy", json!({"min_length": 12, "example": pw}), pw.as_str()),
            ("api_key_policy", json!({"key": key}), key),
            ("password_store", json!({"value": pw}), pw.as_str()),
            ("credentials", json!({"user": "ana", "pw": pw}), pw.as_str()),
        ] {
            let whole = json!({ k: v });
            assert_eq!(r.redact_value(&whole, &mut Stats::default()), json!({ k: REDACTED }), "{whole}");
            assert_gone(&whole.to_string(), secret);
        }
        for ok in [json!({"password_length": 12}), json!({"password_format": "regex"}), json!({"token_type": "bearer"})] {
            assert_eq!(r.redact_value(&ok, &mut Stats::default()), ok, "{ok}");
            assert_eq!(red(&ok.to_string()), ok.to_string(), "{ok}");
        }
    }

    /// A lifetime written in parts (`1h30m`, `168h0m0s`) or in a word (`7days`) is a duration; a namespace of plain
    /// words (`my_app`, `MyApp`, `eu-west-1`) is a name. Letters and digits mixed in one part are not.
    #[test]
    fn compound_durations_and_plain_namespaces_stay() {
        for ok in [
            "token_lifetime: 1h30m",
            "token_lifetime: 168h0m0s",
            "token_lifetime: 7days",
            "secret_namespace: my_app",
            "secret_namespace: MyApp",
            "secret_namespace: eu-west-1",
            "{\"secret_namespace\": \"team-2\"}",
        ] {
            assert_eq!(red(ok), ok, "{ok}");
        }
        for (t, secret) in [
            ("token_lifetime: 1h30x", "1h30x"),
            ("token_lifetime: 7dayz", "7dayz"),
            ("secret_namespace: ns-4f9a2", "ns-4f9a2"),
            ("secret_namespace: Prod-Secret", "Prod-Secret"),
        ] {
            assert_gone(t, secret);
        }
        let r = Redactor::default();
        // (a namespace starts with a letter: a number is not a name; a lifetime is numbers with units, not a unit alone)
        for (k, v) in [
            ("secret_namespace", "Prod_Secret"),
            ("token_lifetime", "1h30x"),
            ("api_key_namespace", "ns-4f9a2"),
            ("secret_namespace", "48291375"),
            ("token_lifetime", "ms"),
        ] {
            assert_eq!(r.redact_value(&json!({ k: v }), &mut Stats::default())[k], REDACTED, "{k}: {v}");
        }
        for (k, v) in [("secret_namespace", "my_app"), ("token_lifetime", "168h0m0s"), ("token_lifetime", "3600")] {
            assert_eq!(r.redact_value(&json!({ k: v }), &mut Stats::default())[k], v, "{k}: {v}");
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
