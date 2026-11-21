//! Hospedeiro de conectores MCP locais de terceiros (servidores MCP por stdio: node, python, binários).
//!
//! Cada conector é descrito por um manifesto e roda isolado, como o usuário, dentro de `kalango-agentes.slice`:
//! - cgroup próprio com limites de memória, CPU, processos (e tempo, quando declarado);
//! - Landlock: lê o sistema, a pasta de instalação e o que o manifesto declarar; escreve só na própria pasta de
//!   dados e no que declarar; a casa do usuário fica fora, inclusive ~/.ssh;
//! - rede: nenhuma (seccomp nega sockets de internet e Landlock nega TCP), ou só as portas declaradas, ou livre;
//! - seccomp: sem ptrace, montagem, módulos, eBPF, namespaces, chaves do kernel e io_uring;
//! - ambiente limpo: só PATH, LANG e o que o manifesto declarar (tokens do usuário não vazam).
//!
//! `kalangod conector testar MANIFESTO` sobe o conector isolado, faz o aperto de mão MCP, lista as ferramentas,
//! sugere a permissão de cada uma pelo tipo e encerra. `kalangod conector rodar MANIFESTO` é a ponte stdio usada
//! pelo motor de agentes: repassa as mensagens, esconde e recusa as ferramentas bloqueadas e registra cada chamada
//! num arquivo que o conector não alcança.
//!
//! Conectores remotos (manifesto com `url`) usam MCP por HTTP com OAuth: veja `remoto.rs`. O filtro de ferramentas e o
//! registro são os mesmos.

use kalango_i18n::{tr, trf};
use crate::limits::{self, Landlock, Limits, Pedido, Seccomp};
use crate::tools::admin::{SISTEMA_ESCRITA, SISTEMA_LEITURA};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifesto {
    pub nome: String,
    /// Conector local: programa a executar.
    #[serde(default)]
    pub comando: String,
    /// Conector remoto: endpoint MCP por HTTP (https; http só em 127.0.0.1/localhost).
    pub url: Option<String>,
    /// Remoto: identificador do cliente OAuth, quando o servidor não aceita registro automático.
    pub cliente_id: Option<String>,
    /// Remoto: escopos OAuth pedidos.
    #[serde(default)]
    pub escopos: Vec<String>,
    /// Remoto sem OAuth: token fixo (use `cofre:NOME`).
    pub token: Option<String>,
    #[serde(default)]
    pub argumentos: Vec<String>,
    /// Pasta de instalação do conector (leitura e execução).
    pub pasta: Option<String>,
    /// Rede: falso = nenhuma.
    #[serde(default)]
    pub rede: bool,
    /// Com rede: se não vazio, só conecta (TCP) nestas portas.
    #[serde(default)]
    pub portas: Vec<u16>,
    #[serde(default)]
    pub leitura: Vec<String>,
    #[serde(default)]
    pub escrita: Vec<String>,
    /// Variáveis de ambiente entregues ao conector (o resto do ambiente não passa).
    #[serde(default)]
    pub ambiente: BTreeMap<String, String>,
    #[serde(default)]
    pub ferramentas_bloqueadas: Vec<String>,
    #[serde(default = "limites_conector")]
    pub limites: Limits,
    /// The agent starts it only when a request needs it (true, the default) or together with itself (false).
    #[serde(default = "sim")]
    pub sob_demanda: bool,
}

fn sim() -> bool {
    true
}

fn limites_conector() -> Limits {
    Limits { memoria_mb: 1024, cpu_pct: 100, processos: 64, tempo_s: 0 }
}

impl Manifesto {
    pub fn load(path: &Path) -> Result<Manifesto, String> {
        let t = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let m: Manifesto = toml::from_str(&t).map_err(|e| format!("{}: {e}", path.display()))?;
        let ok = !m.nome.is_empty() && m.nome.len() <= 64 && m.nome.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_".contains(c));
        if !ok {
            return Err(trf("invalid connector name: {name} (use lowercase letters, digits, - and _)", &[("name", format!("{:?}", m.nome))]));
        }
        match (&m.url, m.comando.is_empty()) {
            (Some(_), false) => return Err(tr("use 'comando' (local connector) or 'url' (remote), not both").into()),
            (None, true) => return Err(tr("missing 'comando' (local connector) or 'url' (remote)").into()),
            (None, false) if m.comando.starts_with('-') => return Err(tr("invalid command").into()),
            (Some(u), _) => {
                crate::remoto::check_url(u)?;
            }
            _ => {}
        }
        if !m.rede && !m.portas.is_empty() {
            return Err(tr("'portas' only applies with 'rede = true'").into());
        }
        Ok(m)
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| "/".into())
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(fallback))
}

fn expand(p: &str) -> PathBuf {
    crate::policy::normalize(&crate::policy::expand(p, &home()))
}

/// Pasta de dados do conector (única onde ele escreve por padrão) e arquivo de registro (fora do alcance dele).
/// The connector's own output (stderr), with times: the agents center shows it in the diagnosis.
pub fn stderr_path(nome: &str) -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join("kalango/conectores").join(format!("{nome}.stderr.log"))
}

fn hora_local() -> String {
    // HH:MM:SS from the local time (libc, no extra crate)
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let mut tm: nix::libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        nix::libc::localtime_r(&(t as nix::libc::time_t), &mut tm);
    }
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

/// A line in the connector's output log (the last 300 lines are kept).
pub fn anotar(nome: &str, linha: &str) {
    let p = stderr_path(nome);
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let mut linhas: Vec<String> = std::fs::read_to_string(&p).unwrap_or_default().lines().map(str::to_string).collect();
    linhas.push(format!("{}  {linha}", hora_local()));
    let k = linhas.len().saturating_sub(300);
    let _ = std::fs::write(&p, linhas[k..].join("\n") + "\n");
}

pub fn paths(nome: &str) -> (PathBuf, PathBuf) {
    (xdg("XDG_DATA_HOME", ".local/share").join("kalango/conectores").join(nome), xdg("XDG_STATE_HOME", ".local/state").join("kalango/conectores").join(format!("{nome}.jsonl")))
}

pub struct Rodando {
    pub child: std::process::Child,
    pub prep_dir: Option<PathBuf>,
    pub isolamento: Value,
}

impl Rodando {
    pub fn stop(&mut self) {
        if !self.prep_dir.as_ref().is_some_and(|d| std::fs::write(d.join("cgroup.kill"), "1").is_ok()) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

/// Sobe o conector isolado, com stdin e stdout ligados a canos.
/// A folder a manifest asks to write that is, holds, or leads (through links) to a startup path: (folder, startup path).
fn startup_write(escrita: &[PathBuf], startup: &[PathBuf]) -> Option<(PathBuf, PathBuf)> {
    use crate::policy::is_within;
    escrita.iter().find_map(|p| {
        let real = crate::sandbox::resolve_existing(p);
        startup.iter().find(|x| {
            let rx = crate::sandbox::resolve_existing(x);
            [p, &real].iter().any(|q| is_within(q, x) || is_within(x, q) || is_within(q, &rx) || is_within(&rx, q))
        }).map(|x| (p.clone(), x.clone()))
    })
}

pub fn spawn(m: &Manifesto) -> Result<Rodando, String> {
    let (dados, registro) = paths(&m.nome);
    std::fs::create_dir_all(dados.join("tmp")).map_err(|e| format!("{}: {e}", dados.display()))?;
    if let Some(d) = registro.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let pasta = m.pasta.as_deref().map(expand);
    let mut leitura: Vec<PathBuf> = SISTEMA_LEITURA.iter().map(Into::into).collect();
    leitura.extend(pasta.iter().cloned());
    leitura.extend(m.leitura.iter().map(|p| expand(p)));
    // /tmp não é compartilhado: cada conector tem o próprio (TMPDIR na pasta de dados)
    let mut escrita: Vec<PathBuf> = SISTEMA_ESCRITA.iter().filter(|p| !p.ends_with("tmp")).map(Into::into).collect();
    escrita.push(dados.clone());
    escrita.extend(m.escrita.iter().map(|p| expand(p)));
    // o Landlock não abre exceção dentro de uma permissão: pedir ~ (ou /home, ou /) daria acesso a ~/.ssh — e pedir
    // ~/.config daria a configuração do KalangoOS (as permissões dos agentes, os outros conectores)
    let mut protegidas: Vec<PathBuf> = [".ssh", ".gnupg", ".local/share/keyrings"].iter().map(|p| home().join(p)).collect();
    // (as written, and in the XDG folders this session uses, where the programs that own them read them)
    for p in crate::policy::KALANGO_PROTECTED {
        protegidas.push(expand(p));
        for (var, prefix) in [("XDG_CONFIG_HOME", "~/.config/"), ("XDG_DATA_HOME", "~/.local/share/"), ("XDG_STATE_HOME", "~/.local/state/")] {
            if let Some(rest) = p.strip_prefix(prefix) {
                protegidas.push(crate::policy::normalize(&xdg(var, &prefix[2..]).join(rest)));
            }
        }
    }
    protegidas.push(crate::cofre::file());
    for p in leitura.iter().chain(&escrita) {
        if let Some(x) = protegidas.iter().find(|x| crate::policy::is_within(p, x) || crate::policy::is_within(x, p)) {
            return Err(trf("the manifest asks for access to {path}, which is or contains the protected folder {protected}", &[("path", p.display().to_string()), ("protected", x.display().to_string())]));
        }
    }
    // a connector that writes where programs start at login would run them as the person, outside its cgroup: only
    // when the person lets agents change those files (startup.rs)
    let setting = xdg("XDG_CONFIG_HOME", ".config").join("kalango/agente-permissoes.json");
    if crate::startup::mode_from_file(&setting) == crate::startup::Mode::Ask {
        let startup: Vec<PathBuf> = crate::policy::STARTUP_PATHS.iter().chain(crate::policy::PROGRAM_FOLDERS).flat_map(|s| crate::startup::expand_glob(&expand(s))).collect();
        if let Some((p, x)) = startup_write(&escrita, &startup) {
            return Err(trf("the manifest asks to write {path}, which is or holds {startup}, a file that runs when you log in", &[("path", p.display().to_string()), ("startup", x.display().to_string())]));
        }
    }
    let portas = if !m.rede { Some(vec![]) } else if m.portas.is_empty() { None } else { Some(m.portas.clone()) };
    let mut ambiente: BTreeMap<String, String> = BTreeMap::new();
    ambiente.insert("PATH".into(), "/usr/local/bin:/usr/bin:/bin".into());
    ambiente.insert("LANG".into(), std::env::var("LANG").unwrap_or_else(|_| "C.UTF-8".into()));
    ambiente.insert("HOME".into(), dados.display().to_string());
    ambiente.insert("TMPDIR".into(), dados.join("tmp").display().to_string());
    ambiente.insert("KALANGO_CONECTOR".into(), m.nome.clone());
    for (k, v) in &m.ambiente {
        // `cofre:NOME` vira o valor guardado, só aqui e só para este conector
        ambiente.insert(k.clone(), crate::cofre::resolve(v)?);
    }
    let isolamento = json!({
        "leitura": leitura, "escrita": escrita,
        "rede": match &portas { Some(p) if p.is_empty() => json!("nenhuma"), Some(p) => json!({"portas_tcp": p}), None => json!("livre") },
        "limites": m.limites, "ambiente": ambiente.keys().collect::<Vec<_>>(), "registro": registro,
    });
    let uid = nix::unistd::getuid().as_raw();
    let gid = nix::unistd::getgid().as_raw();
    let mut lim = m.limites.clone();
    if lim.tempo_s == 0 {
        lim.tempo_s = 365 * 24 * 3600; // conector é de longa duração: sem prazo, salvo se declarado
    }
    let prep = limits::prepare_command(Pedido {
        programa: &m.comando, argumentos: &m.argumentos, uid, gid, limites: lim,
        landlock: Some(Landlock { leitura, escrita, listar: vec![], portas }), seccomp: Some(Seccomp { sem_rede: !m.rede, sem_unix: false }), ambiente: Some(ambiente),
        pasta: Some(pasta.unwrap_or(dados)), dry_run: false,
    }, "kalango-conector")?;
    let mut cmd = prep.cmd;
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    anotar(&m.nome, &trf("starting: {command}", &[("command", format!("{} {}", m.comando, m.argumentos.join(" ")))]));
    let mut child = cmd.spawn().map_err(|e| trf("start connector: {error}", &[("error", e.to_string())]))?;
    // its output goes to its own log (with times), not to kalangod's
    if let Some(err) = child.stderr.take() {
        let nome = m.nome.clone();
        std::thread::spawn(move || {
            for l in BufReader::new(err).lines().map_while(Result::ok) {
                anotar(&nome, &l);
            }
        });
    }
    let isolamento = json!({"modo": prep.modo, "cgroup": prep.dir, "regras": isolamento});
    Ok(Rodando { child, prep_dir: prep.dir, isolamento })
}

/// Permissão sugerida pelo tipo da ferramenta (anotações MCP; sem anotação, o MCP manda supor o pior caso).
fn suggestion(t: &Value) -> &'static str {
    let a = &t["annotations"];
    if a["readOnlyHint"] == true {
        tr("free (read only)")
    } else if a["destructiveHint"] == false {
        tr("ask")
    } else {
        tr("always ask (may destroy or did not declare)")
    }
}

/// `kalangod conector testar`: aperto de mão e lista de ferramentas, com prazo.
pub fn test(path: &Path) -> Result<Value, String> {
    let m = Manifesto::load(path)?;
    if m.url.is_some() {
        return crate::remoto::test(&m);
    }
    let start = Instant::now();
    let mut r = spawn(&m)?;
    let mut stdin = r.child.stdin.take().unwrap();
    let stdout = r.child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for l in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(l).is_err() {
                break;
            }
        }
    });
    let mut tempos: Vec<(&str, u64)> = vec![("processo", start.elapsed().as_millis() as u64)];
    let mut ask = |id: u64, method: &str, params: Value| -> Result<Value, String> {
        writeln!(stdin, "{}", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).map_err(|e| trf("the connector closed: {error}", &[("error", e.to_string())]))?;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = rx.recv_timeout(left).map_err(|e| match e {
                // the process ended before answering (a missing command, a crash): said at once, not after 15 s
                std::sync::mpsc::RecvTimeoutError::Disconnected => tr("the process ended before answering").to_string(),
                _ => trf("the connector did not answer {method} within 15 s", &[("method", method.to_string())]),
            })?;
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            if v["id"] == json!(id) {
                return match v.get("error") {
                    Some(e) => Err(format!("{method}: {}", e["message"])),
                    None => Ok(v["result"].clone()),
                };
            }
        }
    };
    let res = (|| {
        let t0 = Instant::now();
        let init = ask(1, "initialize", json!({"protocolVersion": crate::server::PROTOCOL, "capabilities": {}, "clientInfo": {"name": "kalangod-teste", "version": env!("CARGO_PKG_VERSION")}}))?;
        let t_init = t0.elapsed().as_millis() as u64;
        let t1 = Instant::now();
        let list = ask(2, "tools/list", json!({}))?;
        Ok::<_, String>((init, list, t_init, t1.elapsed().as_millis() as u64))
    })();
    // how it ended, when it ended on its own (its output closes a moment before its exit status is there)
    let saida = if res.is_err() {
        let fim = Instant::now() + Duration::from_secs(1);
        loop {
            match r.child.try_wait() {
                Ok(Some(st)) => break st.code(),
                Ok(None) if Instant::now() < fim => std::thread::sleep(Duration::from_millis(20)),
                _ => break None,
            }
        }
    } else {
        None
    };
    r.stop();
    let (init, list, t_init, t_list) = match res {
        Ok(x) => x,
        Err(e) => {
            if let Some(c) = saida {
                anotar(&m.nome, &trf("process ended with code {code}", &[("code", c.to_string())]));
                return Err(format!("{e} ({})", trf("code {code}", &[("code", c.to_string())])));
            }
            anotar(&m.nome, &e);
            return Err(e);
        }
    };
    tempos.push(("initialize", t_init));
    tempos.push(("tools/list", t_list));
    let mut v = report(&m, &init, &list, r.isolamento.clone(), start);
    v["tempos_ms"] = json!(tempos.into_iter().map(|(k, t)| json!({"etapa": k, "ms": t})).collect::<Vec<_>>());
    Ok(v)
}

/// Relatório do `testar`: servidor, ferramentas com a permissão sugerida e o isolamento.
pub fn report(m: &Manifesto, init: &Value, list: &Value, isolamento: Value, start: Instant) -> Value {
    let bloqueadas: HashSet<&str> = m.ferramentas_bloqueadas.iter().map(String::as_str).collect();
    let ferramentas: Vec<Value> = list["tools"].as_array().cloned().unwrap_or_default().iter().map(|t| {
        let nome = t["name"].as_str().unwrap_or("");
        let leitura = t["annotations"]["readOnlyHint"] == true;
        json!({"nome": nome, "descricao": t["description"], "permissao_sugerida": if bloqueadas.contains(nome) { tr("blocked by the manifest") } else { suggestion(t) },
            // without the annotation MCP says to assume the worst
            "leitura": leitura, "destrutiva": !leitura && t["annotations"]["destructiveHint"] != false})
    }).collect();
    json!({
        "conector": m.nome, "servidor": init["serverInfo"], "protocolo": init["protocolVersion"], "ferramentas": ferramentas, "sob_demanda": m.sob_demanda,
        "isolamento": isolamento, "tempo_ms": start.elapsed().as_millis() as u64,
    })
}

/// Registro de um conector (JSON por linha), fora do alcance dele.
pub struct Registro(Mutex<std::fs::File>);

impl Registro {
    pub fn open(nome: &str) -> Result<Registro, String> {
        let (_, path) = paths(nome);
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        Ok(Registro(Mutex::new(std::fs::OpenOptions::new().create(true).append(true).open(&path).map_err(|e| e.to_string())?)))
    }

    pub fn add(&self, mut v: Value) {
        v["hora"] = json!(crate::timeline::now());
        if let Ok(mut f) = self.0.lock() {
            let _ = writeln!(f, "{v}");
        }
    }
}

/// O que fazer com uma mensagem vinda do cliente.
pub enum Decisao {
    Repassar,
    /// Responder direto ao cliente, sem passar ao conector.
    Responder(Value),
}

/// Filtro comum aos conectores locais e remotos: esconde e recusa ferramentas bloqueadas e registra chamadas.
pub struct Filtro {
    bloqueadas: HashSet<String>,
    pendentes: Mutex<BTreeMap<String, (String, String)>>,
    pub registro: Registro,
}

impl Filtro {
    pub fn new(m: &Manifesto) -> Result<Filtro, String> {
        Ok(Filtro { bloqueadas: m.ferramentas_bloqueadas.iter().cloned().collect(), pendentes: Mutex::default(), registro: Registro::open(&m.nome)? })
    }

    pub fn do_cliente(&self, v: &Value) -> Decisao {
        let method = v["method"].as_str().unwrap_or("");
        if method == "tools/call" {
            let tool = v["params"]["name"].as_str().unwrap_or("").to_string();
            if self.bloqueadas.contains(&tool) {
                self.registro.add(json!({"evento": "recusada", "ferramenta": tool}));
                return Decisao::Responder(json!({"jsonrpc": "2.0", "id": v["id"], "error": {"code": -32601, "message": trf("tool '{tool}' blocked by the connector manifest", &[("tool", tool.clone())])}}));
            }
            self.registro.add(json!({"evento": "chamada", "ferramenta": tool, "argumentos": v["params"]["arguments"]}));
            self.pendentes.lock().unwrap().insert(v["id"].to_string(), (method.into(), tool));
        } else if method == "tools/list" {
            self.pendentes.lock().unwrap().insert(v["id"].to_string(), (method.into(), String::new()));
        }
        Decisao::Repassar
    }

    pub fn do_servidor(&self, mut v: Value) -> Value {
        let key = v["id"].to_string();
        if let Some((method, tool)) = self.pendentes.lock().unwrap().remove(&key) {
            if method == "tools/list" {
                if let Some(tools) = v["result"]["tools"].as_array_mut() {
                    tools.retain(|t| !self.bloqueadas.contains(t["name"].as_str().unwrap_or("")));
                }
            } else {
                let erro = v.get("error").is_some() || v["result"]["isError"] == true;
                self.registro.add(json!({"evento": "resultado", "ferramenta": tool, "erro": erro}));
            }
        }
        v
    }
}

/// Saída para o cliente, compartilhada entre threads.
pub type Saida = Arc<Mutex<std::io::Stdout>>;

pub fn emit(out: &Saida, line: &str) -> bool {
    let mut o = out.lock().unwrap();
    writeln!(o, "{line}").and_then(|_| o.flush()).is_ok()
}

/// `kalangod conector rodar`: ponte stdio entre o motor de agentes e o conector (local isolado, ou remoto).
pub fn run(path: &Path) -> Result<i32, String> {
    let m = Manifesto::load(path)?;
    if m.url.is_some() {
        return crate::remoto::run(&m);
    }
    let mut r = spawn(&m)?;
    let filtro = Arc::new(Filtro::new(&m)?);
    let out: Saida = Arc::new(Mutex::new(std::io::stdout()));
    let mut to_child = r.child.stdin.take().unwrap();
    let from_child = r.child.stdout.take().unwrap();
    filtro.registro.add(json!({"evento": "inicio", "isolamento": r.isolamento}));

    let back = {
        let (out, filtro) = (out.clone(), filtro.clone());
        std::thread::spawn(move || {
            for line in BufReader::new(from_child).lines().map_while(Result::ok) {
                let line = match serde_json::from_str::<Value>(&line) {
                    Ok(v) => filtro.do_servidor(v).to_string(),
                    Err(_) => line,
                };
                if !emit(&out, &line) {
                    break;
                }
            }
        })
    };
    // cliente → conector numa thread própria; quando o cliente fecha, o conector recebe fim de entrada e sai
    {
        let (out, filtro) = (out.clone(), filtro.clone());
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    if let Decisao::Responder(resp) = filtro.do_cliente(&v) {
                        emit(&out, &resp.to_string());
                        continue;
                    }
                }
                if writeln!(to_child, "{line}").is_err() {
                    break;
                }
            }
            drop(to_child);
        });
    }
    // a saída do conector acabou: ele terminou (sozinho, por erro ou pelo limite de memória); a ponte termina junto
    let _ = back.join();
    let code = match r.child.wait() {
        Ok(s) => s.code().unwrap_or_else(|| 128 + std::os::unix::process::ExitStatusExt::signal(&s).unwrap_or(0)),
        Err(_) => 1,
    };
    filtro.registro.add(json!({"evento": "fim", "codigo": code}));
    r.stop();
    if code != 0 {
        eprintln!("kalangod conector: {}", trf("'{name}' exited with code {code}", &[("name", m.nome.clone()), ("code", code.to_string())]));
    }
    Ok(code)
}

#[cfg(test)]
mod startup_tests {
    use super::startup_write;

    /// A manifest's write folder may not be, hold or lead to a startup path (also through a link, also dangling).
    #[test]
    fn a_manifest_cannot_write_where_programs_start() {
        let d = tempfile::tempdir().unwrap();
        let h = d.path();
        std::fs::create_dir_all(h.join(".config/autostart")).unwrap();
        std::fs::create_dir_all(h.join("dados")).unwrap();
        let startup = vec![h.join(".config/autostart"), h.join(".bashrc")];
        assert!(startup_write(&[h.join(".config/autostart")], &startup).is_some());
        assert!(startup_write(&[h.join(".config")], &startup).is_some(), "a folder that holds one");
        std::os::unix::fs::symlink(h.join(".config/autostart"), h.join("dados/saida")).unwrap();
        assert!(startup_write(&[h.join("dados/saida")], &startup).is_some(), "a link to autostart");
        std::os::unix::fs::symlink(h.join(".bashrc"), h.join("dados/rc")).unwrap();
        assert!(startup_write(&[h.join("dados/rc")], &startup).is_some(), "a link to a file not created yet");
        assert!(startup_write(&[h.join("dados")], &startup).is_none(), "the connector's own folder");
    }
}
