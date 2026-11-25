//! What the internet-trigger screens say (design page "37 · Gatilho pela internet"), as plain values with no GTK in
//! them: the tunnel state the bar's chip reads (`$XDG_RUNTIME_DIR/kalango/tunel.json`, written by kalango-fluxosd),
//! the words of the chip and of the quick menu, the levels of a trigger and what each one warns, the rows of the
//! call log and its filters, and the address and the example of a signed call. The windows only draw these.

use kalango_i18n::{tr, trf};
use serde_json::Value;
use std::path::PathBuf;

/// The tunnel as the service's state file describes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tunnel {
    pub open: bool,
    /// "quick" | "named"
    pub mode: String,
    pub url: String,
    pub opened_at: i64,
    pub last_activity: i64,
    pub idle_close_s: Option<u64>,
    pub healthy: bool,
    pub connections: u32,
    /// the triggers it was approved for
    pub triggers: Vec<TunnelTrigger>,
    pub closed_reason: String,
    /// approved and starting
    pub opening: bool,
    pub opening_since: i64,
    /// why the last opening failed: (kind, message)
    pub failure: Option<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TunnelTrigger {
    pub id: String,
    pub flow: String,
    pub signed: bool,
    pub level: String,
}

/// The file the shell reads: written whole by the service, so it is read whole.
pub fn state_file() -> PathBuf {
    if let Some(p) = std::env::var_os("KALANGO_TUNEL_ESTADO").map(PathBuf::from).filter(|p| p.is_absolute()) {
        return p;
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(gtk::glib::user_runtime_dir);
    runtime.join("kalango/tunel.json")
}

pub fn parse_tunnel(text: &str) -> Option<Tunnel> {
    let v: Value = serde_json::from_str(text).ok()?;
    Some(Tunnel {
        open: v["open"].as_bool()?,
        mode: v["mode"].as_str().unwrap_or("").to_string(),
        url: v["url"].as_str().unwrap_or("").to_string(),
        opened_at: v["opened_at"].as_i64().unwrap_or(0),
        last_activity: v["last_activity"].as_i64().unwrap_or(0),
        idle_close_s: v["idle_close_s"].as_u64(),
        healthy: v["healthy"].as_bool().unwrap_or(false),
        connections: v["connections"].as_u64().unwrap_or(0) as u32,
        triggers: v["triggers"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| TunnelTrigger {
                id: t["id"].as_str().unwrap_or("").to_string(),
                flow: t["flow"].as_str().unwrap_or("").to_string(),
                // (anything but a plain `true` is shown as not signed: the safe reading is the alarming one)
                signed: t["signed"].as_bool() == Some(true),
                level: t["level"].as_str().unwrap_or("").to_string(),
            })
            .collect(),
        closed_reason: v["closed_reason"].as_str().unwrap_or("").to_string(),
        opening: v["opening"].as_bool() == Some(true),
        opening_since: v["opening_since"].as_i64().unwrap_or(0),
        failure: v["failure"]["kind"].as_str().map(|k| (k.to_string(), v["failure"]["message"].as_str().unwrap_or("").to_string())),
    })
}

/// `00:42:10` (hours, minutes, seconds).
pub fn elapsed(secs: i64) -> String {
    let s = secs.max(0);
    format!("{:02}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
}

/// The chip in the bar: "túnel aberto · 2 gatilhos" and the time open; “abrindo túnel…” and the time waiting while it
/// starts. None when there is no tunnel to show.
pub fn chip(t: &Tunnel, now: i64) -> Option<(String, String)> {
    if t.opening && !t.open {
        return Some((tr("opening tunnel…").to_string(), elapsed(now - t.opening_since)));
    }
    if !t.open {
        return None;
    }
    let n = t.triggers.len();
    let text = match n {
        0 => tr("tunnel open").to_string(),
        1 => tr("tunnel open · 1 trigger").to_string(),
        n => trf("tunnel open · {n} triggers", &[("n", n.to_string())]),
    };
    Some((text, elapsed(now - t.opened_at)))
}

/// The address without the scheme (as the quick menu and the approval show it).
pub fn host(url: &str) -> &str {
    url.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/')
}

/// "3 min", "1 h", "12 s": how long ago, in the unit that fits.
pub fn ago(secs: i64) -> String {
    let s = secs.max(0);
    if s < 60 {
        trf("{n} s", &[("n", s.to_string())])
    } else if s < 3600 {
        trf("{n} min", &[("n", (s / 60).to_string())])
    } else if s < 86400 {
        trf("{n} h", &[("n", (s / 3600).to_string())])
    } else {
        trf("{n} d", &[("n", (s / 86400).to_string())])
    }
}

/// One call of the log, as `internet.calls` gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub trigger_id: String,
    pub at: i64,
    pub status: i64,
    pub outcome: String,
    pub method: String,
    pub body_bytes: i64,
    pub run_id: Option<String>,
    pub client: String,
    pub body_sha256: String,
    pub run_seconds: Option<i64>,
    pub run_status: String,
}

pub fn parse_calls(v: &Value) -> Vec<Call> {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|r| Call {
            trigger_id: r["trigger_id"].as_str().unwrap_or("").to_string(),
            at: r["at"].as_i64().unwrap_or(0),
            status: r["status"].as_i64().unwrap_or(0),
            outcome: r["outcome"].as_str().unwrap_or("").to_string(),
            method: r["method"].as_str().unwrap_or("").to_string(),
            body_bytes: r["body_bytes"].as_i64().unwrap_or(0),
            run_id: r["run_id"].as_str().map(str::to_string),
            client: r["client"].as_str().unwrap_or("").to_string(),
            body_sha256: r["body_sha256"].as_str().unwrap_or("").to_string(),
            run_seconds: r["run_seconds"].as_i64(),
            run_status: r["run_status"].as_str().unwrap_or("").to_string(),
        })
        .collect()
}

/// "1,2 KB" (the decimal comma of the interface's language is the catalog's: here the number with one decimal).
pub fn size(bytes: i64) -> String {
    if bytes < 1000 {
        trf("{n} B", &[("n", bytes.to_string())])
    } else {
        trf("{n} KB", &[("n", format!("{:.1}", bytes as f64 / 1000.0).replace('.', &decimal()))])
    }
}

fn decimal() -> String {
    tr("decimal point").to_string()
}

/// The line under a call in the quick menu: "última chamada há 3 min · 202", with the reason after a refused one.
pub fn last_call(c: Option<&Call>, now: i64) -> String {
    let Some(c) = c else { return tr("no calls yet").to_string() };
    let why = match signature(c) {
        Sig::Failed | Sig::Stale | Sig::Repeated if c.status == 401 || c.status == 409 => format!(" ({})", tr("signature")),
        _ => String::new(),
    };
    trf("last call {ago} ago · {status}{why}", &[("ago", ago(now - c.at)), ("status", c.status.to_string()), ("why", why)])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sig {
    Ok,
    Failed,
    /// the date was off by more than the window
    Stale,
    Repeated,
    /// refused before the signature was looked at
    NotChecked,
}

/// What the call log says about the signature: it was checked only when the call got that far.
pub fn signature(c: &Call) -> Sig {
    match c.outcome.as_str() {
        "started" => Sig::Ok,
        "signature_required" | "invalid_signature" | "no_key" => Sig::Failed,
        "timestamp_outside_window" | "bad_timestamp" => Sig::Stale,
        "replay" | "replay_cache_full" => Sig::Repeated,
        // refused by a limit that comes before the signature, or after it (the signature was fine)
        "queue_full" | "flow_disabled" | "invalid_json" | "not_text" | "flow_not_runnable" => Sig::Ok,
        _ => Sig::NotChecked,
    }
}

pub fn signature_text(s: Sig) -> String {
    match s {
        Sig::Ok => tr("ok").to_string(),
        Sig::Failed => tr("failed").to_string(),
        Sig::Stale => tr("failed (old date)").to_string(),
        Sig::Repeated => tr("repeated").to_string(),
        Sig::NotChecked => "—".to_string(),
    }
}

/// The last column: what became of the flow.
pub fn flow_text(c: &Call, rate_per_minute: u32) -> String {
    match c.outcome.as_str() {
        "started" => match (c.run_status.as_str(), c.run_seconds) {
            ("done", Some(s)) => trf("ran · {s} s", &[("s", s.to_string())]),
            ("failed" | "blocked" | "stopped" | "cancelled", _) => tr("ran and failed").to_string(),
            _ => tr("running").to_string(),
        },
        "rate_limited" => trf("limit {n}/min", &[("n", rate_per_minute.to_string())]),
        "unknown_path" => tr("wrong path").to_string(),
        "disabled_trigger" => tr("trigger off").to_string(),
        "queue_full" => tr("queue full").to_string(),
        _ => tr("did not run").to_string(),
    }
}

/// The filter chips of the log: ("Todas · 46", all), ("202 · 38", 202)… and the signature one.
#[derive(Clone, Debug, PartialEq)]
pub enum Filter {
    All,
    Status(i64),
    SignatureFailed,
}

pub fn filters(calls: &[Call]) -> Vec<(String, Filter)> {
    let mut out = vec![(trf("All · {n}", &[("n", calls.len().to_string())]), Filter::All)];
    let mut codes: Vec<i64> = calls.iter().map(|c| c.status).collect();
    codes.sort_unstable();
    codes.dedup();
    for code in codes {
        let n = calls.iter().filter(|c| c.status == code).count();
        out.push((format!("{code} · {n}"), Filter::Status(code)));
    }
    if calls.iter().any(|c| matches!(signature(c), Sig::Failed | Sig::Stale)) {
        out.push((tr("Signature failed").to_string(), Filter::SignatureFailed));
    }
    out
}

pub fn matches(f: &Filter, c: &Call) -> bool {
    match f {
        Filter::All => true,
        Filter::Status(s) => c.status == *s,
        Filter::SignatureFailed => matches!(signature(c), Sig::Failed | Sig::Stale),
    }
}

/// The origin as the log shows it: a short hash (never an address), `a91f…3c`.
pub fn origin(hash: &str) -> String {
    if hash.len() >= 6 { format!("{}…{}", &hash[..4], &hash[hash.len() - 2..]) } else { hash.to_string() }
}

/// The body's hash in groups, as the detail sheet shows it.
pub fn body_hash(h: &str) -> String {
    h.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join(" ")
}

// ── the levels of a trigger ────────────────────────────────────────────────────────────────────────────────

/// (value, label): the three choices of "Nível deste gatilho" (the fourth, "total", is not offered here).
pub fn levels() -> Vec<(&'static str, String)> {
    vec![("observar", tr("Lowest (observe)").to_string()), ("usuario", tr("User").to_string()), ("sistema", tr("Administrator").to_string())]
}

/// The note under the levels: the standing one for the lowest, a warning for the others.
pub fn level_note(level: &str) -> (bool, String) {
    match level {
        "observar" => (
            false,
            tr("Data from outside runs at the lowest level: the flow can read the request and tell you, but actions on the system ask for approval.").to_string(),
        ),
        "usuario" => (
            true,
            tr("Data that arrives from outside will be able to act as the User level (read and move your files) in this flow. Changing the level asks for approval and goes to the Timeline.").to_string(),
        ),
        _ => (
            true,
            tr("Data that arrives from outside will be able to act as the Administrator in this flow. Changing the level asks for approval and goes to the Timeline.").to_string(),
        ),
    }
}

pub fn level_warning_title(level: &str) -> String {
    match level {
        "observar" => String::new(),
        _ => tr("Level above the lowest").to_string(),
    }
}

/// Whether the choice of a level asks for approval (only a rise does).
pub fn rises(from: &str, to: &str) -> bool {
    let rank = |l: &str| ["observar", "usuario", "sistema", "total"].iter().position(|x| *x == l).unwrap_or(0);
    rank(to) > rank(from)
}

/// The text of the public (unsigned) warning.
pub fn public_warning() -> (String, String) {
    (
        tr("Anyone with the address can fire this flow").to_string(),
        tr("Without a signature there is no way to know who called. Use it only for tests; the level stays at the lowest and the limit drops to 5 calls per minute.").to_string(),
    )
}

// ── the address and the signed example ─────────────────────────────────────────────────────────────────────────

/// The headers' names as the interface shows them.
pub const HEADER_SIGNATURE: &str = "X-Kalango-Assinatura";
pub const HEADER_TIME: &str = "X-Kalango-Hora";
pub const HEADER_NONCE: &str = "X-Kalango-Nonce";

/// The example of a signed call in the language chosen: the key is a variable of the environment, never written.
pub fn example(lang: &str, url: &str) -> String {
    match lang {
        "python" => format!(
            "import hashlib, hmac, json, os, time, urllib.request\n\
             corpo = json.dumps({{\"nome\": \"Ana\", \"servico\": \"logo\"}}).encode()\n\
             hora = str(int(time.time()))\n\
             assinatura = hmac.new(os.environ[\"CHAVE\"].encode(), hora.encode() + b\".0..\" + corpo, hashlib.sha256).hexdigest()\n\
             req = urllib.request.Request(\"{url}\", data=corpo, method=\"POST\", headers={{\n\
             \x20   \"Content-Type\": \"application/json\", \"{HEADER_TIME}\": hora, \"{HEADER_SIGNATURE}\": \"sha256=\" + assinatura}})\n\
             print(urllib.request.urlopen(req).read().decode())"
        ),
        "javascript" => format!(
            "import {{ createHmac }} from \"node:crypto\";\n\
             const corpo = JSON.stringify({{ nome: \"Ana\", servico: \"logo\" }});\n\
             const hora = String(Math.floor(Date.now() / 1000));\n\
             const assinatura = createHmac(\"sha256\", process.env.CHAVE).update(`${{hora}}.0..${{corpo}}`).digest(\"hex\");\n\
             const r = await fetch(\"{url}\", {{ method: \"POST\", body: corpo, headers: {{\n\
             \x20 \"Content-Type\": \"application/json\", \"{HEADER_TIME}\": hora, \"{HEADER_SIGNATURE}\": `sha256=${{assinatura}}` }} }});\n\
             console.log(r.status, await r.text());"
        ),
        _ => format!(
            "CORPO='{{\"nome\":\"Ana\",\"servico\":\"logo\"}}'\n\
             HORA=$(date +%s)\n\
             NONCE=$(uuidgen)\n\
             ASS=$(printf '%s.%s.%s.%s' \"$HORA\" \"${{#NONCE}}\" \"$NONCE\" \"$CORPO\" \\\n\
             \x20 | openssl dgst -sha256 -hmac \"$CHAVE\" -hex | cut -d' ' -f2)\n\n\
             curl -X POST {url} \\\n\
             \x20 -H 'Content-Type: application/json' \\\n\
             \x20 -H \"{HEADER_TIME}: $HORA\" \\\n\
             \x20 -H \"{HEADER_SIGNATURE}: sha256=$ASS\" \\\n\
             \x20 -H \"{HEADER_NONCE}: $NONCE\" \\\n\
             \x20 -d \"$CORPO\""
        ),
    }
}

// ── the settings page ──────────────────────────────────────────────────────────────────────────────────────────

/// "conectado · 2 conexões ativas · porta local 47911", or why it is not.
pub fn connection_line(t: &Tunnel, port: Option<u64>) -> String {
    if !t.open {
        return tr("closed").to_string();
    }
    let conn = match t.connections {
        0 => tr("opening").to_string(),
        1 => tr("connected · 1 active connection").to_string(),
        n => trf("connected · {n} active connections", &[("n", n.to_string())]),
    };
    match port {
        Some(p) => trf("{conn} · local port {port}", &[("conn", conn), ("port", p.to_string())]),
        None => conn,
    }
}

/// What a failed opening says and what to do (the design's “Falhou ao abrir”): (title, text, tone, can retry).
/// `tone` is "ouro" for what the person can fix by installing something, "coral" for a program that failed its check.
pub fn failure_card(kind: &str) -> (String, String, &'static str) {
    match kind {
        "not_installed" => (
            tr("cloudflared is not installed").to_string(),
            tr("Install it from the App Center (verified package) and try again.").to_string(),
            "ouro",
        ),
        "verification_failed" => (
            tr("The check of the program failed").to_string(),
            tr("The signature of cloudflared does not match: the program was blocked and the tunnel did not open.").to_string(),
            "coral",
        ),
        "denied" => (tr("The tunnel was not opened").to_string(), tr("You did not allow it. Nothing is reachable from the internet.").to_string(), "ouro"),
        _ => (tr("The tunnel did not open").to_string(), tr("Something stopped it; see the details and try again.").to_string(), "ouro"),
    }
}

/// Calls refused for going over the limit in the last minute: (title, text), None when there were none. `by_minute` is
/// what the chart under it draws.
pub fn rate_summary(calls: &[Call], now: i64, per_minute: u32) -> Option<(String, String)> {
    let refused = calls.iter().filter(|c| c.status == 429 && now - c.at <= 60).count();
    if refused == 0 {
        return None;
    }
    Some((
        trf("429 · too many calls · {n} refused in the last minute", &[("n", refused.to_string())]),
        trf("It went over {limit} calls per minute: the next ones get 429 until the window empties. If it is legitimate, raise this trigger's limit.", &[("limit", per_minute.to_string())]),
    ))
}

/// Calls per minute over the last `minutes` (oldest first), for the small chart.
pub fn by_minute(calls: &[Call], now: i64, minutes: usize) -> Vec<u32> {
    let mut v = vec![0u32; minutes];
    for c in calls {
        let ago = ((now - c.at).max(0) / 60) as usize;
        if ago < minutes {
            v[minutes - 1 - ago] += 1;
        }
    }
    v
}

/// The “desligado” card of a trigger that is off: (title, text).
pub fn off_card(disabled_at: Option<i64>, refused: i64, when: &str) -> (String, String) {
    let text = match (disabled_at, refused) {
        (Some(_), 0) => trf("Turned off by you on {when}.", &[("when", when.to_string())]),
        (Some(_), n) => trf("Turned off by you on {when} · {n} calls refused since then (404)", &[("when", when.to_string()), ("n", n.to_string())]),
        (None, _) => tr("The address still exists, but it answers 404 and the flow does not run.").to_string(),
    };
    (tr("Over the internet · off").to_string(), text)
}

/// How many runs go on after the kill switch.
pub fn inflight_row(n: i64) -> (String, String) {
    (tr("Calls in progress").to_string(), if n == 0 { tr("none").to_string() } else { trf("{n} · they go on and end on their own", &[("n", n.to_string())]) })
}

/// The text of the kill switch's banner and of its confirmation.
pub fn kill_title(open: usize) -> String {
    match open {
        1 => tr("Close the open tunnel").to_string(),
        _ => tr("Close all tunnels").to_string(),
    }
}

pub fn kill_confirm_rows(t: &Tunnel, inflight: i64) -> Vec<(String, String)> {
    let (k, v) = inflight_row(inflight);
    vec![
        (tr("Tunnel").to_string(), if t.open { tr("1 open").to_string() } else { tr("none open").to_string() }),
        (tr("Triggers").to_string(), trf("{n} · will stop answering", &[("n", t.triggers.len().to_string())])),
        (k, v),
    ]
}

/// The menu's title: one tunnel carries all the triggers.
pub fn menu_title(t: &Tunnel) -> String {
    match t.mode.as_str() {
        "named" => tr("Tunnel open · my domain").to_string(),
        _ => tr("Tunnel open · quick").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn t() -> Tunnel {
        parse_tunnel(
            &json!({"open": true, "mode": "quick", "url": "https://exemplo-gatilho.trycloudflare.com", "opened_at": 1000, "last_activity": 2500,
                "idle_close_s": 7200, "healthy": true, "connections": 2,
                "triggers": [{"id": "gi-1", "flow": "Orçamentos do site", "signed": true, "level": "observar"}, {"id": "gi-2", "flow": "Formulário de contato", "signed": true, "level": "observar"}]})
            .to_string(),
        )
        .unwrap()
    }

    #[test]
    fn the_state_file_is_read_whole_and_a_broken_one_is_not_a_tunnel() {
        let v = t();
        assert!(v.open && v.healthy && v.connections == 2 && v.triggers.len() == 2);
        assert_eq!(host(&v.url), "exemplo-gatilho.trycloudflare.com");
        assert!(parse_tunnel("{").is_none());
        assert!(parse_tunnel("{\"mode\": \"quick\"}").is_none(), "without `open` it says nothing");
        let closed = parse_tunnel(&json!({"open": false, "closed_reason": "idle", "triggers": []}).to_string()).unwrap();
        assert!(!closed.open && closed.closed_reason == "idle");
        assert!(chip(&closed, 5000).is_none(), "the chip is only there while a tunnel is open");
    }

    #[test]
    fn the_chip_counts_the_triggers_and_the_time() {
        let (text, time) = chip(&t(), 1000 + 42 * 60 + 10).unwrap();
        assert!(text.contains('2'), "{text}");
        assert_eq!(time, "00:42:10");
        assert_eq!(elapsed(3661), "01:01:01");
        assert_eq!(elapsed(-5), "00:00:00");
        let mut one = t();
        one.triggers.truncate(1);
        assert!(!chip(&one, 1000).unwrap().0.contains('2'));
    }

    #[test]
    fn calls_read_as_the_log_shows_them() {
        let v = json!([
            {"at": 100, "status": 202, "outcome": "started", "method": "POST", "body_bytes": 1214, "run_id": "r1", "client": "a91f00003c", "body_sha256": "4f1c9a07e3b211d8", "run_seconds": 4, "run_status": "done"},
            {"at": 90, "status": 401, "outcome": "timestamp_outside_window", "method": "POST", "body_bytes": 1100, "run_id": null, "client": "7be000091", "body_sha256": "ab", "run_seconds": null, "run_status": null},
            {"at": 80, "status": 401, "outcome": "invalid_signature", "method": "POST", "body_bytes": 1100, "run_id": null, "client": "7be000091", "body_sha256": "ab"},
            {"at": 70, "status": 429, "outcome": "rate_limited", "method": "POST", "body_bytes": 0, "run_id": null, "client": "03cd0aa", "body_sha256": null}
        ]);
        let calls = parse_calls(&v);
        assert_eq!(calls.len(), 4);
        assert_eq!(signature(&calls[0]), Sig::Ok);
        assert_eq!(signature(&calls[1]), Sig::Stale);
        assert_eq!(signature(&calls[2]), Sig::Failed);
        assert_eq!(signature(&calls[3]), Sig::NotChecked, "refused by a limit before the signature was looked at");
        assert!(flow_text(&calls[0], 30).contains('4'));
        assert!(flow_text(&calls[3], 30).contains("30"));
        assert_eq!(flow_text(&calls[1], 30), tr("did not run"));
        assert_eq!(origin("a91f00003c"), "a91f…3c");
        assert_eq!(body_hash("4f1c9a07e3b211d8"), "4f1c 9a07 e3b2 11d8");
        assert!(size(1214).contains("1"), "{}", size(1214));
    }

    #[test]
    fn the_filters_count_what_is_in_the_log() {
        let calls = parse_calls(&json!([
            {"at": 1, "status": 202, "outcome": "started"}, {"at": 2, "status": 202, "outcome": "started"},
            {"at": 3, "status": 401, "outcome": "invalid_signature"}, {"at": 4, "status": 429, "outcome": "rate_limited"}]));
        let f = filters(&calls);
        let labels: Vec<&str> = f.iter().map(|(l, _)| l.as_str()).collect();
        assert!(labels[0].contains('4'), "{labels:?}");
        assert!(labels.contains(&"202 · 2") && labels.contains(&"401 · 1") && labels.contains(&"429 · 1"), "{labels:?}");
        assert!(f.iter().any(|(_, x)| *x == Filter::SignatureFailed));
        let only: Vec<&Call> = calls.iter().filter(|c| matches(&Filter::Status(202), c)).collect();
        assert_eq!(only.len(), 2);
        assert_eq!(calls.iter().filter(|c| matches(&Filter::SignatureFailed, c)).count(), 1);
        // no failed signature in the log: no such chip
        assert!(!filters(&calls[..2]).iter().any(|(_, x)| *x == Filter::SignatureFailed));
    }

    #[test]
    fn the_last_call_line_says_why_a_refusal_happened() {
        let calls = parse_calls(&json!([{"at": 100, "status": 401, "outcome": "invalid_signature"}, {"at": 100, "status": 202, "outcome": "started"}]));
        let refused = last_call(Some(&calls[0]), 100 + 3600);
        assert!(refused.contains("401") && refused.contains(tr("signature")), "{refused}");
        let ok = last_call(Some(&calls[1]), 100 + 180);
        assert!(ok.contains("202") && !ok.contains(tr("signature")), "{ok}");
        assert_eq!(last_call(None, 0), tr("no calls yet"));
        assert_eq!(ago(30), trf("{n} s", &[("n", "30".into())]));
        assert_eq!(ago(180), trf("{n} min", &[("n", "3".into())]));
        assert_eq!(ago(7200), trf("{n} h", &[("n", "2".into())]));
    }

    #[test]
    fn only_the_lowest_level_has_no_warning_and_a_rise_is_what_asks() {
        assert!(!level_note("observar").0);
        assert!(level_note("usuario").0 && level_note("sistema").0);
        assert!(level_warning_title("observar").is_empty() && !level_warning_title("usuario").is_empty());
        assert!(rises("observar", "usuario") && rises("usuario", "sistema"));
        assert!(!rises("usuario", "observar") && !rises("usuario", "usuario"), "lowering never asks");
        assert_eq!(levels().iter().map(|(v, _)| *v).collect::<Vec<_>>(), vec!["observar", "usuario", "sistema"]);
        let (title, body) = public_warning();
        assert!(!title.is_empty() && body.contains('5'));
    }

    #[test]
    fn the_examples_never_hold_a_key_and_use_the_headers_of_the_design() {
        for lang in ["curl", "python", "javascript"] {
            let e = example(lang, "https://exemplo-gatilho.trycloudflare.com/h/abc");
            assert!(e.contains(HEADER_SIGNATURE) && e.contains(HEADER_TIME), "{lang}");
            assert!(e.contains("CHAVE") && !e.contains("kg_sk"), "{lang}: the key comes from the environment");
            assert!(e.contains("https://exemplo-gatilho.trycloudflare.com/h/abc"));
        }
        assert!(example("curl", "u").contains(HEADER_NONCE));
        // the message that is signed: timestamp.LENGTH.nonce.body (no nonce: timestamp.0..body)
        assert!(example("curl", "u").contains("${#NONCE}"));
        assert!(example("python", "u").contains(".0..") && example("javascript", "u").contains(".0.."));
    }

    #[test]
    fn the_states_of_the_design_say_what_they_mean() {
        // opening: the chip says so, with the time waiting; closed and not opening: nothing
        let opening = parse_tunnel(&json!({"open": false, "opening": true, "opening_since": 1000}).to_string()).unwrap();
        let (text, time) = chip(&opening, 1018).unwrap();
        assert_eq!((text, time), (tr("opening tunnel…").to_string(), "00:00:18".to_string()));
        assert!(chip(&parse_tunnel("{\"open\": false}").unwrap(), 5).is_none());
        // failed: what each kind says and what to do
        let failed = parse_tunnel(&json!({"open": false, "failure": {"kind": "not_installed", "message": "x"}}).to_string()).unwrap();
        assert_eq!(failed.failure.as_ref().map(|f| f.0.as_str()), Some("not_installed"));
        assert_eq!(failure_card("not_installed").2, "ouro");
        assert_eq!(failure_card("verification_failed").2, "coral");
        assert_ne!(failure_card("not_installed").0, failure_card("verification_failed").0);
        // the 429 card: only with refusals in the last minute; the chart counts per minute
        let calls = parse_calls(&json!([{"at": 990, "status": 429}, {"at": 995, "status": 429}, {"at": 400, "status": 429}, {"at": 990, "status": 202}]));
        let (title, text) = rate_summary(&calls, 1000, 30).unwrap();
        assert!(title.contains('2') && text.contains("30"), "{title} / {text}");
        assert!(rate_summary(&calls, 5000, 30).is_none(), "an old burst is not news");
        let bars = by_minute(&calls, 1000, 10);
        assert_eq!(bars.len(), 10);
        assert_eq!((bars[9], bars[8]), (3, 0));
        // a trigger turned off
        let (t, x) = off_card(Some(1), 3, "24/11 22:10");
        assert!(t.contains(tr("off")) || !t.is_empty());
        assert!(x.contains('3') && x.contains("24/11 22:10"), "{x}");
        assert!(!off_card(Some(1), 0, "agora").1.contains("404"));
        // the menu says it is one tunnel
        assert_ne!(menu_title(&Tunnel { mode: "named".into(), ..Default::default() }), menu_title(&Tunnel { mode: "quick".into(), ..Default::default() }));
    }

    #[test]
    fn the_connection_line_says_what_the_page_shows() {
        let v = t();
        let l = connection_line(&v, Some(47911));
        assert!(l.contains("47911") && l.contains('2'), "{l}");
        assert_eq!(connection_line(&Tunnel::default(), None), tr("closed"));
        assert_eq!(kill_title(2), tr("Close all tunnels"));
        assert_eq!(kill_title(1), tr("Close the open tunnel"));
        assert_eq!(kill_confirm_rows(&v, 1).len(), 3);
        assert!(kill_confirm_rows(&v, 1)[2].1.contains('1') && kill_confirm_rows(&v, 0)[2].1 == tr("none"));
    }
}
