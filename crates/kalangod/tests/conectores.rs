//! Conectores MCP de terceiros isolados: um conector de teste tenta ler, escrever, conectar, depurar e vazar o
//! ambiente; o hospedeiro (`kalangod conector`) precisa permitir só o que o manifesto declara.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_kalangod");

struct Casa {
    dir: tempfile::TempDir,
}

impl Casa {
    fn new() -> Casa {
        let dir = tempfile::tempdir().unwrap();
        let h = dir.path().join("casa");
        std::fs::create_dir_all(h.join(".ssh")).unwrap();
        std::fs::write(h.join(".ssh/id_ed25519"), "CHAVE").unwrap();
        std::fs::create_dir_all(h.join("docs")).unwrap();
        std::fs::write(h.join("docs/a.txt"), "documento").unwrap();
        std::fs::create_dir_all(h.join("outros")).unwrap();
        std::fs::write(h.join("outros/b.txt"), "não declarado").unwrap();
        Casa { dir }
    }
    fn home(&self) -> PathBuf {
        self.dir.path().join("casa")
    }
    fn manifesto(&self, extra: &str) -> PathBuf {
        let fx = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let p = self.dir.path().join("conector.toml");
        std::fs::write(&p, format!(
            "nome = \"teste\"\ncomando = \"/usr/bin/python3\"\nargumentos = [\"-I\", \"{}/conector_teste.py\"]\npasta = \"{}\"\nferramentas_bloqueadas = [\"perigosa\"]\nambiente = {{ DECLARADA = \"sim\" }}\n{extra}\n",
            fx.display(), fx.display())).unwrap();
        p
    }
    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(BIN);
        c.env("LANGUAGE", "pt_BR"); // the assertions check the Portuguese text
        c.args(args).env("HOME", self.home()).env("XDG_DATA_HOME", self.dir.path().join("dados")).env("XDG_STATE_HOME", self.dir.path().join("estado"))
            .env("SEGREDO_TESTE", "token-do-usuario");
        c
    }
    fn registro(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("estado/kalango/conectores/teste.jsonl")).unwrap_or_default()
    }
}

struct Ponte {
    child: Child,
    r: BufReader<std::process::ChildStdout>,
    n: u64,
}

impl Ponte {
    fn new(casa: &Casa, manifesto: &Path) -> Ponte {
        let mut child = casa.cmd(&["conector", "rodar", manifesto.to_str().unwrap()]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let r = BufReader::new(child.stdout.take().unwrap());
        let mut p = Ponte { child, r, n: 0 };
        assert!(p.rpc("initialize", json!({"protocolVersion": "2025-06-18"})).get("result").is_some());
        p
    }
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.n += 1;
        let w = self.child.stdin.as_mut().unwrap();
        writeln!(w, "{}", json!({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params})).unwrap();
        w.flush().unwrap();
        let mut line = String::new();
        self.r.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or(Value::Null)
    }
    /// (erro?, texto)
    fn call(&mut self, tool: &str, args: Value) -> (bool, String) {
        let v = self.rpc("tools/call", json!({"name": tool, "arguments": args}));
        if let Some(e) = v.get("error") {
            return (true, e["message"].as_str().unwrap_or("").to_string());
        }
        (v["result"]["isError"] == true, v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string())
    }
}

impl Drop for Ponte {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.wait();
    }
}

fn user_systemd() -> bool {
    Command::new("systemd-run").args(["--user", "--scope", "--quiet", "true"]).status().map(|s| s.success()).unwrap_or(false)
}

#[test]
fn testar_lista_ferramentas_e_sugere_permissoes() {
    if !user_systemd() {
        return;
    }
    let casa = Casa::new();
    let m = casa.manifesto("");
    let out = casa.cmd(&["conector", "testar", m.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["servidor"]["name"], "conector-teste");
    let sug = |n: &str| v["ferramentas"].as_array().unwrap().iter().find(|t| t["nome"] == n).unwrap()["permissao_sugerida"].as_str().unwrap().to_string();
    assert!(sug("ler").starts_with("livre"), "{v}");
    assert_eq!(sug("escrever"), "perguntar");
    assert!(sug("conectar").starts_with("perguntar sempre"), "sem anotação, supõe o pior");
    assert_eq!(sug("perigosa"), "bloqueada pelo manifesto");
    assert_eq!(v["isolamento"]["regras"]["rede"], "nenhuma");
}

#[test]
fn isolamento_do_conector() {
    if !user_systemd() {
        return;
    }
    let casa = Casa::new();
    let m = casa.manifesto("leitura = [\"~/docs\"]\n[limites]\nmemoria_mb = 128\nprocessos = 32\ncpu_pct = 100\ntempo_s = 0");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let porta = listener.local_addr().unwrap().port();
    let mut p = Ponte::new(&casa, &m);

    // ferramenta bloqueada: some da lista e é recusada sem chegar ao conector
    let lista = p.rpc("tools/list", json!({}));
    let nomes: Vec<&str> = lista["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(nomes.contains(&"ler") && !nomes.contains(&"perigosa"), "{nomes:?}");
    let (erro, txt) = p.call("perigosa", json!({}));
    assert!(erro && txt.contains("bloqueada"), "{txt}");

    // arquivos: lê só o declarado; ~/.ssh e o resto da casa ficam fora; escreve só na própria pasta de dados
    let h = casa.home();
    assert_eq!(p.call("ler", json!({"caminho": h.join("docs/a.txt")})), (false, "documento".into()));
    let (erro, txt) = p.call("ler", json!({"caminho": h.join("outros/b.txt")}));
    assert!(erro && txt.contains("Permission denied"), "{txt}");
    let (erro, txt) = p.call("ler", json!({"caminho": h.join(".ssh/id_ed25519")}));
    assert!(erro && txt.contains("Permission denied"), "{txt}");
    assert_eq!(p.call("escrever", json!({"caminho": "~/meu.txt", "conteudo": "x"})), (false, "ok".into()), "HOME do conector é a pasta de dados");
    assert!(casa.dir.path().join("dados/kalango/conectores/teste/meu.txt").exists());
    let (erro, txt) = p.call("escrever", json!({"caminho": h.join("docs/novo.txt"), "conteudo": "x"}));
    assert!(erro && txt.contains("Permission denied"), "leitura declarada não vira escrita: {txt}");
    let (erro, _) = p.call("escrever", json!({"caminho": "/tmp/kalango-conector-teste.txt", "conteudo": "x"}));
    assert!(erro, "o /tmp compartilhado fica fora");

    // rede: nenhuma
    let (erro, txt) = p.call("conectar", json!({"porta": porta}));
    assert!(erro && txt.contains("Operation not permitted"), "{txt}");

    // ambiente limpo: o token do usuário não vaza, o declarado chega
    let (_, env) = p.call("ambiente", json!({}));
    assert!(!env.contains("SEGREDO_TESTE") && env.contains("DECLARADA") && env.contains("TMPDIR"), "{env}");

    // cgroup dos agentes, seccomp e exec permitido
    let (_, cg) = p.call("cgroup", json!({}));
    assert!(cg.contains("/kalango.slice/kalango-agentes.slice/kalango-conector-"), "{cg}");
    assert_eq!(p.call("ptrace", json!({})).1, "-1 1", "ptrace negado pelo seccomp (EPERM)");
    assert_eq!(p.call("executar", json!({"programa": ["/bin/echo", "oi"]})).1, "oi\n");

    // registro fora do alcance do conector
    let reg = casa.registro();
    assert!(reg.contains("\"evento\":\"recusada\"") && reg.contains("\"ferramenta\":\"ler\"") && reg.contains("\"evento\":\"resultado\""), "{reg}");

    // memória: passar do limite derruba só o conector; a ponte termina com erro
    let v = p.rpc("tools/call", json!({"name": "memoria", "arguments": {"mb": 600}}));
    assert!(v.is_null(), "o conector devia ter sido encerrado pelo limite de memória: {v}");
    drop(listener);
}

#[test]
fn rede_so_nas_portas_declaradas() {
    if !user_systemd() {
        return;
    }
    let casa = Casa::new();
    let a = TcpListener::bind("127.0.0.1:0").unwrap();
    let b = TcpListener::bind("127.0.0.1:0").unwrap();
    let (pa, pb) = (a.local_addr().unwrap().port(), b.local_addr().unwrap().port());
    let m = casa.manifesto(&format!("rede = true\nportas = [{pa}]"));
    let mut p = Ponte::new(&casa, &m);
    assert_eq!(p.call("conectar", json!({"porta": pa})), (false, "ok".into()));
    let (erro, txt) = p.call("conectar", json!({"porta": pb}));
    assert!(erro && txt.contains("Permission denied"), "porta não declarada: {txt}");
    drop(p);
    // rede livre
    let m = casa.manifesto("rede = true");
    let mut p = Ponte::new(&casa, &m);
    assert_eq!(p.call("conectar", json!({"porta": pb})), (false, "ok".into()));
}

#[test]
fn manifesto_nao_pede_pasta_protegida() {
    let casa = Casa::new();
    for extra in ["leitura = [\"~\"]", "escrita = [\"~/.ssh\"]", "leitura = [\"/\"]", "leitura = [\"~/../casa\"]"] {
        let m = casa.manifesto(extra);
        let out = casa.cmd(&["conector", "testar", m.to_str().unwrap()]).output().unwrap();
        assert!(!out.status.success(), "{extra} devia ser recusado");
        assert!(String::from_utf8_lossy(&out.stderr).contains("protegida"), "{extra}: {}", String::from_utf8_lossy(&out.stderr));
    }
    let m = casa.manifesto("portas = [443]");
    let out = casa.cmd(&["conector", "testar", m.to_str().unwrap()]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("portas"), "portas sem rede é erro");
}

#[test]
fn segredo_do_cofre_chega_so_ao_conector() {
    if !user_systemd() {
        return;
    }
    let casa = Casa::new();
    let mut g = casa.cmd(&["cofre", "guardar", "token-teste"]).env("KALANGO_COFRE", "arquivo").stdin(Stdio::piped()).spawn().unwrap();
    writeln!(g.stdin.take().unwrap(), "valor-secreto").unwrap();
    assert!(g.wait().unwrap().success());
    let m = casa.manifesto("");
    let t = std::fs::read_to_string(&m).unwrap().replace("DECLARADA = \"sim\"", "DECLARADA = \"cofre:token-teste\"");
    std::fs::write(&m, t).unwrap();
    let mut c = casa.cmd(&["conector", "rodar", m.to_str().unwrap()]);
    c.env("KALANGO_COFRE", "arquivo").stdin(Stdio::piped()).stdout(Stdio::piped());
    let mut child = c.spawn().unwrap();
    let r = BufReader::new(child.stdout.take().unwrap());
    let mut p = Ponte { child, r, n: 0 };
    p.rpc("initialize", json!({}));
    assert_eq!(p.call("valor", json!({"nome": "DECLARADA"})), (false, "valor-secreto".into()));
    // e o conector não alcança o arquivo do cofre
    let (erro, txt) = p.call("ler", json!({"caminho": casa.dir.path().join("dados/kalango/cofre.json")}));
    assert!(erro && txt.contains("Permission denied"), "{txt}");
    // segredo inexistente: o conector nem sobe
    let t = std::fs::read_to_string(&m).unwrap().replace("cofre:token-teste", "cofre:nao-existe");
    std::fs::write(&m, t).unwrap();
    let o = casa.cmd(&["conector", "testar", m.to_str().unwrap()]).env("KALANGO_COFRE", "arquivo").output().unwrap();
    assert!(String::from_utf8_lossy(&o.stderr).contains("não está no cofre"));
}

fn landlock_abi() -> i64 {
    unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, std::ptr::null::<u8>(), 0usize, 1u32) }.max(0)
}

/// A connector cannot reach the person's session bus (nor a stand-in socket of another program) and cannot have
/// systemd run anything for it — the VirtualDesk review's escape: `systemd-run --user` through /run/user/UID/bus
/// landed as the person outside every rule. With Landlock ABI 9 the kernel keeps it off sockets it did not make;
/// older kernels give it no local sockets at all.
#[test]
fn a_connector_cannot_reach_the_session_bus_nor_run_systemd() {
    if !user_systemd() {
        return;
    }
    let casa = Casa::new();
    let outro = casa.dir.path().join("outro.sock");
    let _l = std::os::unix::net::UnixListener::bind(&outro).unwrap();
    let uid = nix::unistd::getuid().as_raw();
    let m = casa.manifesto("");
    let mut p = Ponte::new(&casa, &m);
    let script = format!("import socket\nfor a in ['/run/user/{uid}/bus', '{}']:\n    try:\n        s = socket.socket(socket.AF_UNIX); s.connect(a); print('CONECTOU', a)\n    except OSError as e:\n        print('recusado', e.errno)\n", outro.display());
    let (_, out) = p.call("executar", json!({"programa": ["/usr/bin/python3", "-c", script]}));
    assert!(!out.contains("CONECTOU") && out.matches("recusado").count() == 2, "{out}");
    let (_, out) = p.call("executar", json!({"programa": ["/bin/sh", "-c", format!("XDG_RUNTIME_DIR=/run/user/{uid} DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/{uid}/bus systemd-run --user --quiet --scope /bin/true 2>&1; echo codigo=$?")]}));
    assert!(out.contains("codigo=") && !out.contains("codigo=0"), "systemd-run --user must fail: {out}");
}

/// A socket the connector needs (a database's) is declared in its manifest, by the person, and it is the only other
/// one it reaches — where the kernel can keep it to that (Landlock ABI 9); elsewhere such a connector does not start.
/// A socket through which programs get started (the session bus) is never given, whatever the manifest says.
#[test]
fn a_connectors_declared_socket_and_nothing_else() {
    if !user_systemd() {
        return;
    }
    let casa = Casa::new();
    let banco = casa.dir.path().join("banco.sock");
    let outro = casa.dir.path().join("outro.sock");
    let _a = std::os::unix::net::UnixListener::bind(&banco).unwrap();
    let _b = std::os::unix::net::UnixListener::bind(&outro).unwrap();
    let uid = nix::unistd::getuid().as_raw();
    // the session bus: refused before anything runs
    if Path::new(&format!("/run/user/{uid}/bus")).exists() {
        let m = casa.manifesto(&format!("sockets = [\"/run/user/{uid}/bus\"]"));
        let o = casa.cmd(&["conector", "testar", m.to_str().unwrap()]).output().unwrap();
        let txt = String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr);
        assert!(txt.contains("fora do isolamento"), "{txt}");
    }
    let m = casa.manifesto(&format!("sockets = [\"{}\"]", banco.display()));
    if landlock_abi() < 9 {
        let o = casa.cmd(&["conector", "testar", m.to_str().unwrap()]).output().unwrap();
        let txt = String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr);
        assert!(txt.contains("ABI 9"), "without Landlock ABI 9 it does not start: {txt}");
        return;
    }
    let mut p = Ponte::new(&casa, &m);
    let script = format!("import socket\nfor a in ['{}', '{}']:\n    try:\n        s = socket.socket(socket.AF_UNIX); s.connect(a); print('conectou', a)\n    except OSError as e:\n        print('recusado', a, e.errno)\n", banco.display(), outro.display());
    let (_, out) = p.call("executar", json!({"programa": ["/usr/bin/python3", "-c", script]}));
    assert!(out.contains(&format!("conectou {}", banco.display())) && out.contains(&format!("recusado {}", outro.display())), "{out}");
}
