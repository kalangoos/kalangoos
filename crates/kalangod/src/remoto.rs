//! Conectores MCP remotos: MCP por HTTP ("Streamable HTTP", 2025-06-18) com autorização OAuth 2.1.
//!
//! - `kalangod conector entrar MANIFESTO`: descobre o servidor de autorização (metadados do recurso, RFC 9728, e do
//!   servidor, RFC 8414), registra o KalangoOS como cliente quando o servidor permite (RFC 7591), abre o navegador
//!   para você autorizar e recebe o código numa porta local (127.0.0.1), com PKCE (S256), `state` e `resource`
//!   (RFC 8707). Os tokens vão para o cofre, nunca para arquivos de configuração.
//! - `kalangod conector rodar MANIFESTO`: ponte stdio ⇄ HTTP. Repassa cada mensagem, entende respostas JSON e SSE,
//!   mantém a sessão (`Mcp-Session-Id`), renova o token quando vence e aplica o mesmo filtro e registro dos
//!   conectores locais.
//! - Só fala com a origem declarada no manifesto (sem seguir redirecionamentos). HTTPS é obrigatório, exceto em
//!   127.0.0.1/localhost (desenvolvimento e testes).

use kalango_i18n::{tr, trf};
use crate::conector::{emit, report, Decisao, Filtro, Manifesto, Saida};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use url::Url;

pub fn check_url(u: &str) -> Result<Url, String> {
    let url = Url::parse(u).map_err(|e| trf("invalid url {url}: {error}", &[("url", format!("{u:?}")), ("error", e.to_string())]))?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]" | "::1"));
    match url.scheme() {
        "https" => Ok(url),
        "http" if local => Ok(url),
        _ => Err(trf("{url}: use https (http only on 127.0.0.1 or localhost)", &[("url", u.to_string())])),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
    /// instante (unix) em que o token de acesso vence; 0 = não informado
    expira: u64,
    /// instante em que foi emitido (para renovar com folga proporcional à validade)
    #[serde(default)]
    emitido: u64,
    token_endpoint: String,
    client_id: String,
    resource: String,
}

fn vault_key(nome: &str) -> String {
    format!("conector:{nome}:oauth")
}

fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn random(n: usize) -> String {
    let mut buf = vec![0u8; n];
    getrandom::getrandom(&mut buf).expect("getrandom");
    b64(&buf)
}

fn agent() -> ureq::Agent {
    // sem redirecionamentos: a ponte não é levada para outra origem
    ureq::AgentBuilder::new().timeout(Duration::from_secs(120)).redirects(0).build()
}

enum Falha {
    NaoAutorizado,
    Outra(String),
}

fn parse_sse(body: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for ev in body.replace("\r\n", "\n").split("\n\n") {
        let data: Vec<&str> = ev.lines().filter_map(|l| l.strip_prefix("data:")).map(|d| d.strip_prefix(' ').unwrap_or(d)).collect();
        if !data.is_empty() {
            if let Ok(v) = serde_json::from_str(&data.join("\n")) {
                out.push(v);
            }
        }
    }
    out
}

struct Sessao<'a> {
    m: &'a Manifesto,
    url: Url,
    agent: ureq::Agent,
    tokens: Mutex<Option<Tokens>>,
    sessao: Mutex<Option<String>>,
    protocolo: Mutex<String>,
}

impl<'a> Sessao<'a> {
    fn new(m: &'a Manifesto) -> Result<Sessao<'a>, String> {
        let url = check_url(m.url.as_deref().unwrap_or(""))?;
        let tokens = crate::cofre::get(&vault_key(&m.nome))?.and_then(|t| serde_json::from_str(&t).ok());
        Ok(Sessao { m, url, agent: agent(), tokens: Mutex::new(tokens), sessao: Mutex::default(), protocolo: Mutex::new(crate::server::PROTOCOL.into()) })
    }

    fn bearer(&self) -> Result<Option<String>, String> {
        if let Some(t) = &self.m.token {
            return crate::cofre::resolve(t).map(Some);
        }
        // renova um pouco antes de vencer: 30 s, ou um quarto da validade se ela for curta
        let vencido = self.tokens.lock().unwrap().as_ref().is_some_and(|t| {
            let folga = 30.min(t.expira.saturating_sub(t.emitido) / 4);
            t.expira != 0 && crate::timeline::now() + folga >= t.expira && t.refresh_token.is_some()
        });
        if vencido {
            self.refresh()?;
        }
        Ok(self.tokens.lock().unwrap().as_ref().map(|t| t.access_token.clone()))
    }

    fn refresh(&self) -> Result<(), String> {
        let t = self.tokens.lock().unwrap().clone().ok_or(tr("no tokens"))?;
        let rt = t.refresh_token.clone().ok_or(tr("the server gave no refresh token"))?;
        let novo = token_request(&self.agent, &t.token_endpoint, &[("grant_type", "refresh_token"), ("refresh_token", &rt), ("client_id", &t.client_id), ("resource", &t.resource)], &t)
            .map_err(|e| trf("the authorization expired or was revoked ({error}): run `kalangod conector entrar` for the connector '{name}'", &[("error", e.to_string()), ("name", self.m.nome.clone())]))?;
        crate::cofre::put(&vault_key(&self.m.nome), &serde_json::to_string(&novo).unwrap())?;
        *self.tokens.lock().unwrap() = Some(novo);
        Ok(())
    }

    fn post(&self, msg: &Value) -> Result<Vec<Value>, Falha> {
        let mut req = self.agent.post(self.url.as_str())
            .set("Content-Type", "application/json")
            .set("Accept", "application/json, text/event-stream")
            .set("MCP-Protocol-Version", &self.protocolo.lock().unwrap());
        if let Some(s) = self.sessao.lock().unwrap().as_ref() {
            req = req.set("Mcp-Session-Id", s);
        }
        if let Some(b) = self.bearer().map_err(Falha::Outra)? {
            req = req.set("Authorization", &format!("Bearer {b}"));
        }
        match req.send_string(&msg.to_string()) {
            Ok(resp) => {
                if let Some(sid) = resp.header("Mcp-Session-Id") {
                    *self.sessao.lock().unwrap() = Some(sid.to_string());
                }
                if resp.status() == 202 {
                    return Ok(vec![]);
                }
                let sse = resp.content_type().contains("event-stream");
                let body = resp.into_string().map_err(|e| Falha::Outra(e.to_string()))?;
                if sse {
                    return Ok(parse_sse(&body));
                }
                if body.trim().is_empty() {
                    return Ok(vec![]);
                }
                match serde_json::from_str::<Value>(&body).map_err(|e| Falha::Outra(trf("invalid response: {error}", &[("error", e.to_string())])))? {
                    Value::Array(a) => Ok(a),
                    v => Ok(vec![v]),
                }
            }
            Err(ureq::Error::Status(401, _)) => Err(Falha::NaoAutorizado),
            Err(ureq::Error::Status(code, resp)) => Err(Falha::Outra(format!("HTTP {code}: {}", resp.into_string().unwrap_or_default().chars().take(300).collect::<String>()))),
            Err(e) => Err(Falha::Outra(e.to_string())),
        }
    }

    /// Envia uma mensagem; se o token não vale mais, renova uma vez e tenta de novo.
    fn send(&self, msg: &Value) -> Result<Vec<Value>, String> {
        let r = match self.post(msg) {
            Err(Falha::NaoAutorizado) if self.tokens.lock().unwrap().as_ref().is_some_and(|t| t.refresh_token.is_some()) => {
                self.refresh()?;
                self.post(msg)
            }
            r => r,
        };
        let out = match r {
            Ok(v) => v,
            Err(Falha::NaoAutorizado) => return Err(trf("not authorized: run `kalangod conector entrar` for the connector '{name}'", &[("name", self.m.nome.clone())])),
            Err(Falha::Outra(e)) => return Err(e),
        };
        if msg["method"] == "initialize" {
            if let Some(p) = out.iter().find_map(|v| v["result"]["protocolVersion"].as_str()) {
                *self.protocolo.lock().unwrap() = p.to_string();
            }
        }
        Ok(out)
    }

    fn close(&self) {
        if let Some(s) = self.sessao.lock().unwrap().take() {
            let _ = self.agent.delete(self.url.as_str()).set("Mcp-Session-Id", &s).call();
        }
    }
}

fn token_request(agent: &ureq::Agent, endpoint: &str, form: &[(&str, &str)], base: &Tokens) -> Result<Tokens, String> {
    check_url(endpoint)?;
    let v: Value = match agent.post(endpoint).set("Accept", "application/json").send_form(form) {
        Ok(r) => r.into_json().map_err(|e| e.to_string())?,
        Err(ureq::Error::Status(c, r)) => return Err(trf("authorization server answered {code}: {body}", &[("code", c.to_string()), ("body", r.into_string().unwrap_or_default())])),
        Err(e) => return Err(e.to_string()),
    };
    let access = v["access_token"].as_str().ok_or(trf("response without access_token: {response}", &[("response", v.to_string())]))?;
    let expira = v["expires_in"].as_u64().map(|s| crate::timeline::now() + s).unwrap_or(0);
    Ok(Tokens {
        access_token: access.into(),
        // a renovação pode trocar o token de renovação (rotação); se não vier, o antigo continua valendo
        refresh_token: v["refresh_token"].as_str().map(str::to_string).or(base.refresh_token.clone()),
        expira, emitido: crate::timeline::now(), token_endpoint: base.token_endpoint.clone(), client_id: base.client_id.clone(), resource: base.resource.clone(),
    })
}

fn get_json(agent: &ureq::Agent, u: &str) -> Result<Value, String> {
    check_url(u)?;
    agent.get(u).set("Accept", "application/json").call().map_err(|e| format!("{u}: {e}"))?.into_json().map_err(|e| format!("{u}: {e}"))
}

/// URL de metadados "bem conhecida", inserida antes do caminho (RFC 8414 / RFC 9728).
fn well_known(base: &Url, nome: &str) -> String {
    let path = base.path().trim_end_matches('/');
    format!("{}/.well-known/{nome}{}", base.origin().ascii_serialization(), path)
}

/// `kalangod conector entrar`: OAuth com PKCE pela porta local; guarda os tokens no cofre.
pub fn login(m: &Manifesto) -> Result<Value, String> {
    let url = check_url(m.url.as_deref().ok_or(tr("this connector is local: there is nothing to authorize"))?)?;
    let ag = agent();
    // 1. o servidor pede autorização? (401 com o endereço dos metadados do recurso)
    let init = json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": crate::server::PROTOCOL, "capabilities": {}, "clientInfo": {"name": "kalangod"}}});
    let www = match ag.post(url.as_str()).set("Content-Type", "application/json").set("Accept", "application/json, text/event-stream").send_string(&init.to_string()) {
        Ok(_) => return Ok(json!({"conector": m.nome, "autorizado": true, "mensagem": tr("the server does not ask for authorization")})),
        Err(ureq::Error::Status(401, r)) => r.header("WWW-Authenticate").unwrap_or("").to_string(),
        Err(e) => return Err(format!("{url}: {e}")),
    };
    let rm_url = www.split("resource_metadata=\"").nth(1).and_then(|r| r.split('"').next()).map(str::to_string).unwrap_or_else(|| well_known(&url, "oauth-protected-resource"));
    let rm = get_json(&ag, &rm_url)?;
    let issuer = check_url(rm["authorization_servers"][0].as_str().ok_or(tr("resource metadata without authorization_servers"))?)?;
    let asm = get_json(&ag, &well_known(&issuer, "oauth-authorization-server")).or_else(|_| get_json(&ag, &well_known(&issuer, "openid-configuration")))?;
    if let Some(ms) = asm["code_challenge_methods_supported"].as_array() {
        if !ms.iter().any(|x| x == "S256") {
            return Err(tr("the authorization server does not offer PKCE S256: refused").into());
        }
    }
    let auth_ep = check_url(asm["authorization_endpoint"].as_str().ok_or(tr("no authorization_endpoint"))?)?;
    let token_ep = check_url(asm["token_endpoint"].as_str().ok_or(tr("no token_endpoint"))?)?.to_string();
    // 2. porta local para receber o código
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let redirect = format!("http://127.0.0.1:{}/retorno", listener.local_addr().unwrap().port());
    // 3. cliente: o do manifesto ou registro automático
    let client_id = match (&m.cliente_id, asm["registration_endpoint"].as_str()) {
        (Some(c), _) => c.clone(),
        (None, Some(reg)) => {
            check_url(reg)?;
            let r: Value = ag.post(reg).send_json(json!({
                "client_name": "KalangoOS", "redirect_uris": [redirect], "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"], "token_endpoint_auth_method": "none",
            })).map_err(|e| trf("client registration: {error}", &[("error", e.to_string())]))?.into_json().map_err(|e| e.to_string())?;
            r["client_id"].as_str().ok_or(tr("registration without client_id"))?.to_string()
        }
        (None, None) => return Err(tr("the server does not accept automatic registration: set 'cliente_id' in the manifest").into()),
    };
    // 4. PKCE + state, e o navegador
    let verifier = random(32);
    let challenge = b64(&Sha256::digest(verifier.as_bytes()));
    let state = random(16);
    let mut auth = auth_ep.clone();
    {
        let mut q = auth.query_pairs_mut();
        q.append_pair("response_type", "code").append_pair("client_id", &client_id).append_pair("redirect_uri", &redirect)
            .append_pair("code_challenge", &challenge).append_pair("code_challenge_method", "S256").append_pair("state", &state)
            .append_pair("resource", url.as_str());
        if !m.escopos.is_empty() {
            q.append_pair("scope", &m.escopos.join(" "));
        }
    }
    eprintln!("kalangod: {}\n  {auth}", trf("authorize the connector '{name}' in the browser:", &[("name", m.nome.clone())]));
    let opener = std::env::var("KALANGO_ABRIR").unwrap_or_else(|_| "xdg-open".into());
    let _ = std::process::Command::new(&opener).arg(auth.as_str()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
    // 5. espera o retorno (5 min)
    let (code, got_state) = wait_callback(&listener, Duration::from_secs(300))?;
    if got_state != state {
        return Err(tr("the returned 'state' does not match: authorization discarded").into());
    }
    let base = Tokens { access_token: String::new(), refresh_token: None, expira: 0, emitido: 0, token_endpoint: token_ep.clone(), client_id: client_id.clone(), resource: url.to_string() };
    let tokens = token_request(&ag, &token_ep, &[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", &redirect), ("client_id", &client_id), ("code_verifier", &verifier), ("resource", url.as_str())], &base)?;
    let onde = crate::cofre::put(&vault_key(&m.nome), &serde_json::to_string(&tokens).unwrap())?;
    Ok(json!({"conector": m.nome, "autorizado": true, "servidor_de_autorizacao": issuer.as_str(), "cliente": client_id,
        "renovavel": tokens.refresh_token.is_some(), "expira_em_s": tokens.expira.saturating_sub(crate::timeline::now()), "guardado_em": onde}))
}

fn wait_callback(listener: &std::net::TcpListener, max: Duration) -> Result<(String, String), String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let end = Instant::now() + max;
    loop {
        match listener.accept() {
            Ok((mut s, _)) => {
                s.set_nonblocking(false).ok();
                s.set_read_timeout(Some(Duration::from_secs(10))).ok();
                let mut line = String::new();
                BufReader::new(&mut s).read_line(&mut line).map_err(|e| e.to_string())?;
                let path = line.split_whitespace().nth(1).unwrap_or("");
                let u = Url::parse(&format!("http://127.0.0.1{path}")).map_err(|e| e.to_string())?;
                if u.path() != "/retorno" {
                    let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                    continue;
                }
                let q: std::collections::HashMap<_, _> = u.query_pairs().into_owned().collect();
                let ok = q.contains_key("code");
                let body = if ok { "<h2>KalangoOS</h2><p>Conector autorizado. Pode fechar esta aba.</p>" } else { "<h2>KalangoOS</h2><p>Autorização negada.</p>" };
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let mut rest = [0u8; 1];
                let _ = s.read(&mut rest);
                if let Some(e) = q.get("error") {
                    return Err(trf("authorization denied: {error}", &[("error", e.to_string())]));
                }
                return Ok((q.get("code").cloned().unwrap_or_default(), q.get("state").cloned().unwrap_or_default()));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() > end {
                    return Err(tr("timed out waiting for the authorization in the browser").into());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// `kalangod conector testar` de um remoto.
pub fn test(m: &Manifesto) -> Result<Value, String> {
    let start = Instant::now();
    let s = Sessao::new(m)?;
    let one = |msg: Value| -> Result<Value, String> {
        let id = msg["id"].clone();
        let out = s.send(&msg)?;
        let v = out.into_iter().find(|v| v["id"] == id).ok_or(tr("the server did not answer"))?;
        match v.get("error") {
            Some(e) => Err(format!("{}: {}", msg["method"], e["message"])),
            None => Ok(v["result"].clone()),
        }
    };
    let init = one(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": crate::server::PROTOCOL, "capabilities": {}, "clientInfo": {"name": "kalangod-teste"}}}))?;
    let _ = s.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    let list = one(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}))?;
    s.close();
    let auth = if m.token.is_some() { tr("vault token") } else if s.tokens.lock().unwrap().is_some() { "OAuth" } else { tr("none") };
    Ok(report(m, &init, &list, json!({"remoto": s.url.as_str(), "autenticacao": auth}), start))
}

/// `kalangod conector rodar` de um remoto: ponte stdio ⇄ HTTP.
pub fn run(m: &Manifesto) -> Result<i32, String> {
    let s = Sessao::new(m)?;
    let filtro = Arc::new(Filtro::new(m)?);
    let out: Saida = Arc::new(Mutex::new(std::io::stdout()));
    filtro.registro.add(json!({"evento": "inicio", "remoto": s.url.as_str()}));
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        if let Decisao::Responder(resp) = filtro.do_cliente(&v) {
            emit(&out, &resp.to_string());
            continue;
        }
        match s.send(&v) {
            Ok(msgs) => {
                for msg in msgs {
                    emit(&out, &filtro.do_servidor(msg).to_string());
                }
            }
            Err(e) => {
                filtro.registro.add(json!({"evento": "falha", "erro": e}));
                if v.get("id").is_some() {
                    emit(&out, &filtro.do_servidor(json!({"jsonrpc": "2.0", "id": v["id"], "error": {"code": -32000, "message": e}})).to_string());
                }
            }
        }
    }
    s.close();
    filtro.registro.add(json!({"evento": "fim", "codigo": 0}));
    Ok(0)
}
