//! Conectores remotos (MCP por HTTP + OAuth) e o cofre, contra um servidor de teste local que exige OAuth com PKCE,
//! emite tokens de 2 s e responde em SSE.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_kalangod");

fn fx(n: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(n)
}

struct Ambiente {
    dir: tempfile::TempDir,
    servidor: Child,
    porta: u16,
}

impl Drop for Ambiente {
    fn drop(&mut self) {
        let _ = self.servidor.kill();
    }
}

impl Ambiente {
    fn new() -> Ambiente {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("casa")).unwrap();
        let mut servidor = Command::new("python3").arg("-I").arg(fx("servidor_remoto.py")).arg("2").stdout(Stdio::piped()).spawn().unwrap();
        let mut l = String::new();
        BufReader::new(servidor.stdout.take().unwrap()).read_line(&mut l).unwrap();
        Ambiente { dir, servidor, porta: l.trim().parse().unwrap() }
    }
    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(BIN);
        c.env("LANGUAGE", "pt_BR"); // the assertions check the Portuguese text
        c.args(args).env("HOME", self.dir.path().join("casa")).env("XDG_DATA_HOME", self.dir.path().join("dados")).env("XDG_STATE_HOME", self.dir.path().join("estado"))
            .env("KALANGO_COFRE", "arquivo").env("KALANGO_ABRIR", fx("navegador_teste.sh"));
        c
    }
    fn manifesto(&self) -> PathBuf {
        let p = self.dir.path().join("remoto.toml");
        std::fs::write(&p, format!("nome = \"remoto\"\nurl = \"http://127.0.0.1:{}/mcp\"\nferramentas_bloqueadas = [\"perigosa\"]\n", self.porta)).unwrap();
        p
    }
    fn out(&self, args: &[&str]) -> (bool, String, String) {
        let o = self.cmd(args).output().unwrap();
        (o.status.success(), String::from_utf8_lossy(&o.stdout).into(), String::from_utf8_lossy(&o.stderr).into())
    }
    fn ponte(&self, linhas: &[Value], pausa_antes_da_ultima: f32) -> Vec<Value> {
        let m = self.manifesto();
        let mut ch = self.cmd(&["conector", "rodar", m.to_str().unwrap()]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let mut w = ch.stdin.take().unwrap();
        for (i, l) in linhas.iter().enumerate() {
            if i == linhas.len() - 1 && pausa_antes_da_ultima > 0.0 {
                std::thread::sleep(std::time::Duration::from_secs_f32(pausa_antes_da_ultima));
            }
            writeln!(w, "{l}").unwrap();
        }
        drop(w);
        let o = ch.wait_with_output().unwrap();
        String::from_utf8_lossy(&o.stdout).lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }
}

fn call(id: u64, name: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}})
}

#[test]
fn cofre_guarda_lista_e_apaga() {
    let a = Ambiente::new();
    let mut c = a.cmd(&["cofre", "guardar", "minha-chave"]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    writeln!(c.stdin.take().unwrap(), "s3gr3do").unwrap();
    assert!(c.wait().unwrap().success());
    let f = a.dir.path().join("dados/kalango/cofre.json");
    assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(f.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(a.out(&["cofre", "listar"]).1, "minha-chave\n", "lista nomes, nunca valores");
    assert!(a.out(&["cofre", "apagar", "minha-chave"]).1.contains("apagado"));
    assert_eq!(a.out(&["cofre", "listar"]).1, "");
    assert!(!a.out(&["cofre", "guardar", "--x"]).0, "nome que parece opção é recusado");
}

#[test]
fn oauth_sessao_sse_renovacao_e_revogacao() {
    let a = Ambiente::new();
    let m = a.manifesto();
    let ms = m.to_str().unwrap();
    // sem autorização: diz o que fazer
    let (ok, _, err) = a.out(&["conector", "testar", ms]);
    assert!(!ok && err.contains("conector entrar"), "{err}");
    // OAuth com PKCE, registro automático e retorno pela porta local
    let (ok, out, err) = a.out(&["conector", "entrar", ms]);
    assert!(ok, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["autorizado"], true);
    assert_eq!(v["renovavel"], true);
    // o token fica no cofre, nunca no manifesto
    assert!(!std::fs::read_to_string(&m).unwrap().contains("token"));
    assert_eq!(a.out(&["cofre", "listar"]).1, "conector:remoto:oauth\n");
    // testar: ferramentas e sugestões
    let (ok, out, err) = a.out(&["conector", "testar", ms]);
    assert!(ok, "{err}");
    let t: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(t["isolamento"]["autenticacao"], "OAuth");
    assert!(t["ferramentas"].as_array().unwrap().iter().any(|f| f["nome"] == "perigosa" && f["permissao_sugerida"] == "bloqueada pelo manifesto"));
    // ponte: sessão + SSE; o token de 2 s vence no meio e é renovado sozinho
    let r = a.ponte(&[
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        call(3, "eco", json!({"texto": "olá"})),
        call(4, "perigosa", json!({})),
        call(5, "contadores", json!({})),
    ], 2.5);
    let by = |id: u64| r.iter().find(|v| v["id"] == id).cloned().unwrap_or(Value::Null);
    assert_eq!(by(1)["result"]["serverInfo"]["name"], "remoto-teste");
    let lista = by(2);
    let nomes: Vec<&str> = lista["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(nomes, ["eco", "contadores"], "a bloqueada some da lista");
    assert_eq!(by(3)["result"]["content"][0]["text"], "olá");
    assert!(r.iter().any(|v| v["method"] == "notifications/progress"), "eventos SSE repassados");
    assert!(by(4)["error"]["message"].as_str().unwrap().contains("bloqueada"));
    let cont: Value = serde_json::from_str(by(5)["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(cont["renovacoes"].as_u64().unwrap() >= 1 && cont["renovacoes"].as_u64().unwrap() <= 2, "renovou ao vencer, sem renovar a cada pedido: {cont}");
    assert_eq!(cont["chamadas"], 2, "a bloqueada não chegou ao servidor");
    let reg = std::fs::read_to_string(a.dir.path().join("estado/kalango/conectores/remoto.jsonl")).unwrap();
    assert!(reg.contains("\"evento\":\"recusada\"") && reg.contains("\"ferramenta\":\"eco\""));
    // revogado no servidor: a ponte responde com erro claro
    ureq_post(a.porta, "/revogar");
    std::thread::sleep(std::time::Duration::from_millis(2100));
    let r = a.ponte(&[json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})], 0.0);
    assert!(r[0]["error"]["message"].as_str().unwrap().contains("conector entrar"), "{r:?}");
}

fn ureq_post(porta: u16, path: &str) {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", porta)).unwrap();
    write!(s, "POST {path} HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    let mut b = String::new();
    let _ = std::io::Read::read_to_string(&mut s, &mut b);
}

#[test]
fn so_https_fora_de_localhost() {
    let a = Ambiente::new();
    for u in ["http://exemplo.com/mcp", "ftp://127.0.0.1/x"] {
        let p = a.dir.path().join("x.toml");
        std::fs::write(&p, format!("nome = \"x\"\nurl = \"{u}\"\n")).unwrap();
        let (ok, _, err) = a.out(&["conector", "testar", p.to_str().unwrap()]);
        assert!(!ok && err.contains("https"), "{u}: {err}");
    }
    let p = a.dir.path().join("y.toml");
    std::fs::write(&p, "nome = \"y\"\nurl = \"https://exemplo.com/mcp\"\ncomando = \"/bin/true\"\n").unwrap();
    assert!(a.out(&["conector", "testar", p.to_str().unwrap()]).2.contains("não os dois"));
}
