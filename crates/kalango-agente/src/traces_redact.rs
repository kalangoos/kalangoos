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
];

/// Keys that hold a whole environment: their value is dropped.
const ENV_KEYS: &[&str] = &["env", "environ", "environment", "envp", "variaveis_de_ambiente"];

/// A string longer than this is not sent: its size and hash are.
pub const MAX_STRING_BYTES: usize = 64 * 1024;

pub const REDACTED: &str = "[REDACTED]";

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
    key.chars().filter(|c| !matches!(c, '_' | '-' | ' ' | '.')).flat_map(char::to_lowercase).collect()
}

impl Redactor {
    /// A redactor with its own deny list (the tests build ones with a key left out to show each key is needed).
    pub fn new(deny: &[&str]) -> Redactor {
        Redactor { deny: deny.iter().map(|k| normalize(k)).collect() }
    }

    fn denied(&self, key: &str) -> bool {
        let k = normalize(key);
        !k.is_empty() && self.deny.iter().any(|d| k.contains(d.as_str()))
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
                        if self.denied(k) && !matches!(x, Value::Number(_) | Value::Bool(_) | Value::Null) {
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
                    let leaked_secret = self.denied(k) && !matches!(x, Value::Number(_) | Value::Bool(_) | Value::Null);
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
        let mut n = self.scan_text(s);
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
        let s = self.key_values(&s, stats);
        let s = env_dump(&s, stats);
        tokens(&s, stats)
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

    /// `password=hunter2`, `"api_key": "abc"`, `Authorization: Bearer abc`: the value goes, the name stays.
    fn key_values(&self, s: &str, stats: &mut Stats) -> String {
        let b = s.as_bytes();
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] != b'=' && b[i] != b':' {
                i += 1;
                continue;
            }
            // the name before the separator (quotes and spaces between them allowed)
            let mut e = i;
            while e > 0 && matches!(b[e - 1], b'"' | b'\'' | b' ' | b'\t' | b'\\') {
                e -= 1;
            }
            let mut st = e;
            while st > 0 && (b[st - 1].is_ascii_alphanumeric() || matches!(b[st - 1], b'_' | b'-' | b'.')) {
                st -= 1;
            }
            if st == e || !self.denied(&s[st..e]) {
                i += 1;
                continue;
            }
            // the value after it
            let mut vs = i + 1;
            while vs < b.len() && matches!(b[vs], b' ' | b'\t') {
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
            let quote = if vs < b.len() && matches!(b[vs], b'"' | b'\'') { Some(b[vs]) } else { None };
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
                    }
                }
                None => {
                    while ve < b.len() && !b[ve].is_ascii_whitespace() && !matches!(b[ve], b'"' | b'\'' | b',' | b';' | b'&' | b')' | b'}' | b'>') {
                        ve += 1;
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
                        if we > w {
                            ve = we;
                        }
                    }
                }
            }
            let value = &s[vs..ve];
            let numeric = !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit() || c == b'.');
            if value.is_empty() || numeric || value.starts_with("[REDACTED") || matches!(value, "null" | "true" | "false") {
                i = ve.max(i + 1);
                continue;
            }
            ranges.push((vs, ve));
            i = ve.max(i + 1);
        }
        replace_ranges(s, &ranges, REDACTED, stats)
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
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    for &(a, b) in ranges {
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
    if t.len() < 24 || looks_like_uuid(t) || t.bytes().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    let has_digit = t.bytes().any(|c| c.is_ascii_digit());
    let has_alpha = t.bytes().any(|c| c.is_ascii_alphabetic());
    has_digit && has_alpha && shannon(t) >= 4.0
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
        }
    }
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

    #[test]
    fn stats_count_what_was_redacted() {
        let mut st = Stats::default();
        Redactor::default().redact_value(&json!({"password": "x", "t": concat!("ghp", "_abcdefghijklmnopqrstuvwxyz0123456789")}), &mut st);
        assert_eq!(st.redacted, 2);
    }
}
