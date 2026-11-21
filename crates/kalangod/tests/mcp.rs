//! Testes de ponta a ponta: sobe o kalangod em modo de desenvolvimento (sem root, simulando ações de sistema)
//! e conversa com ele pelo protocolo MCP, como um agente faria.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_kalangod");

struct Daemon {
    child: Child,
    sock: PathBuf,
    _dir: tempfile::TempDir,
    area: PathBuf,
    raiz: PathBuf,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn start() -> Daemon {
    start_with(true, "")
}

/// `dry_run` falso executa de verdade (sem root: só o que um usuário pode, numa raiz de teste).
fn start_with(dry_run: bool, extra: &str) -> Daemon {
    start_full(dry_run, extra, |c| c)
}

fn start_cfg(edit: impl Fn(String) -> String) -> Daemon {
    start_full(true, "", edit)
}

fn start_full(dry_run: bool, extra: &str, edit: impl Fn(String) -> String) -> Daemon {
    start_full_env(dry_run, extra, edit, &[])
}

/// With variables for kalangod (stand-ins for system commands).
fn start_full_env(dry_run: bool, extra: &str, edit: impl Fn(String) -> String, env: &[(&str, &str)]) -> Daemon {
    let dir = tempfile::tempdir().unwrap();
    let area = dir.path().join("area");
    std::fs::create_dir_all(area.join("segredo")).unwrap();
    std::fs::write(area.join("segredo/chave"), "não leia").unwrap();
    let sock = dir.path().join("mcp.sock");
    let cfg = format!(r#"
socket = "{sock}"
state_dir = "{state}"
protected_paths = ["{area}/segredo"]
approval_requires_tty = false
approval_timeout_s = 20
total_grant_s = 900
dry_run = {dry_run}
restore_root = "{raiz}"
trash_dir = "{raiz}/lixeira"
{extra}
[[callers]]
name = "teste"
max_level = "total"
allowed_paths = ["{area}"]
"#, sock = sock.display(), state = dir.path().join("estado").display(), area = area.display(), raiz = dir.path().join("raiz").display());
    std::fs::create_dir_all(dir.path().join("raiz/etc")).unwrap();
    let cfg_path = dir.path().join("kalangod.toml");
    std::fs::write(&cfg_path, edit(cfg)).unwrap();
    let child = Command::new(BIN).env("LANGUAGE", "pt_BR").envs(env.iter().copied()).args(["servir", "--config", cfg_path.to_str().unwrap()]).spawn().unwrap();
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let raiz = dir.path().join("raiz");
    Daemon { child, sock, _dir: dir, area, raiz }
}

struct Client {
    w: UnixStream,
    r: BufReader<UnixStream>,
    n: u64,
}

impl Client {
    fn new(sock: &Path) -> Client {
        let s = UnixStream::connect(sock).unwrap();
        Client { w: s.try_clone().unwrap(), r: BufReader::new(s), n: 0 }
    }
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.n += 1;
        writeln!(self.w, "{}", json!({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params})).unwrap();
        let mut line = String::new();
        self.r.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
    fn call(&mut self, name: &str, args: Value) -> Value {
        self.rpc("tools/call", json!({"name": name, "arguments": args}))["result"].clone()
    }
}

/// Aprova o próximo pedido pendente a partir de outro processo (como o usuário faria no terminal).
fn approve_next(sock: PathBuf) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        for _ in 0..200 {
            let mut c = Client::new(&sock);
            let p = c.rpc("kalango/pendentes", json!({}));
            if let Some(id) = p["result"].as_array().and_then(|a| a.first()).and_then(|x| x["id"].as_u64()) {
                let out = Command::new(BIN).args(["aprovar", &id.to_string()]).env("KALANGOD_SOCKET", &sock).output().unwrap();
                return String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        "nenhum pedido".into()
    })
}

#[test]
fn protocolo_e_ferramentas() {
    let d = start();
    let mut c = Client::new(&d.sock);
    let init = c.rpc("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "teste", "version": "1"}}));
    assert_eq!(init["result"]["serverInfo"]["name"], "kalangod");
    let tools = c.rpc("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for n in ["sistema.info", "kernel.seguranca", "arquivos.escrever", "pacotes.instalar", "sistema.executar", "linha_do_tempo.desfazer"] {
        assert!(names.contains(&n), "falta {n}");
    }
    let info = c.call("sistema.info", json!({}));
    assert_eq!(info["isError"], false);
    assert!(info["structuredContent"]["kernel"].as_str().unwrap().len() > 3);
    let k = c.call("kernel.seguranca", json!({}));
    assert!(k["structuredContent"]["landlock_abi"].as_i64().unwrap() >= 1, "Landlock precisa estar ativo: {k}");
    assert_eq!(c.rpc("metodo/inexistente", json!({}))["error"]["code"], -32601);
}

#[test]
fn mover_para_uma_pasta_que_existe_poe_dentro() {
    // as `mv` does: the destination is an existing folder, so the file goes into it, keeping its name; a file
    // already there with that name is still refused (nothing is overwritten)
    let d = start();
    let mut c = Client::new(&d.sock);
    let a = d.area.join("fatura.pdf");
    std::fs::write(&a, "pdf").unwrap();
    std::fs::create_dir_all(d.area.join("PDFs")).unwrap();
    let mv = c.call("arquivos.mover", json!({"de": a, "para": d.area.join("PDFs")}));
    assert_eq!(mv["isError"], false, "{mv}");
    assert!(d.area.join("PDFs/fatura.pdf").exists() && !a.exists());
    assert_eq!(mv["structuredContent"]["para"], json!(d.area.join("PDFs/fatura.pdf")));
    std::fs::write(&a, "outra").unwrap();
    let de_novo = c.call("arquivos.mover", json!({"de": a, "para": d.area.join("PDFs")}));
    assert_eq!(de_novo["isError"], true, "{de_novo}");
    assert_eq!(std::fs::read_to_string(d.area.join("PDFs/fatura.pdf")).unwrap(), "pdf");
}

#[test]
fn arquivos_com_desfazer() {
    let d = start();
    let mut c = Client::new(&d.sock);
    let a = d.area.join("nota.md");
    let r = c.call("arquivos.escrever", json!({"caminho": a, "conteudo": "versão 1"}));
    assert_eq!(r["isError"], false, "{r}");
    c.call("arquivos.escrever", json!({"caminho": a, "conteudo": "versão 2"}));
    let lido = c.call("arquivos.ler", json!({"caminho": a}));
    assert_eq!(lido["structuredContent"]["conteudo"], "versão 2");

    let b = d.area.join("pasta/nota-movida.md");
    let mv = c.call("arquivos.mover", json!({"de": a, "para": b}));
    let id = mv["structuredContent"]["registro"].as_u64().unwrap();
    assert!(b.exists() && !a.exists());

    // desfazer é nível sistema: precisa de aprovação de outro processo
    let h = approve_next(d.sock.clone());
    let u = c.call("linha_do_tempo.desfazer", json!({"id": id}));
    assert!(h.join().unwrap().contains("aprovado"));
    assert_eq!(u["isError"], false, "{u}");
    assert!(a.exists() && !b.exists(), "o arquivo devia voltar ao lugar original");
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "versão 2");

    // desfazer de novo é recusado
    let h = approve_next(d.sock.clone());
    let again = c.call("linha_do_tempo.desfazer", json!({"id": id}));
    h.join().unwrap();
    assert_eq!(again["isError"], true);

    let tl = c.call("linha_do_tempo.listar", json!({}));
    assert!(tl["structuredContent"]["registros"].as_array().unwrap().iter().any(|e| e["status"] == "desfeito"));
}

/// "Desfazer tudo desde…": one approval undoes every reversible action since a moment (newest first), and the records
/// read by the agents center (`kalango/registros`, not recorded itself) say which were undone.
#[test]
fn undo_since_with_one_approval() {
    let d = start();
    let mut c = Client::new(&d.sock);
    let (a, b) = (d.area.join("um.txt"), d.area.join("dois.txt"));
    c.call("arquivos.escrever", json!({"caminho": a, "conteudo": "1"}));
    c.call("arquivos.escrever", json!({"caminho": b, "conteudo": "2"}));
    // (records have one-second resolution: the moves start in a later second than the writes)
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let (a2, b2) = (d.area.join("pasta/um.txt"), d.area.join("pasta/dois.txt"));
    c.call("arquivos.mover", json!({"de": a, "para": a2}));
    let desde = c.rpc("kalango/registros", json!({}))["result"]["registros"].as_array().unwrap().last().unwrap()["time"].as_u64().unwrap();
    c.call("arquivos.mover", json!({"de": b, "para": b2}));
    assert!(a2.exists() && b2.exists());
    let antes = c.rpc("kalango/registros", json!({}))["result"]["registros"].as_array().unwrap().len();
    let h = approve_next(d.sock.clone());
    let u = c.call("linha_do_tempo.desfazer_desde", json!({"desde": desde}));
    assert!(h.join().unwrap().contains("aprovado"), "one approval");
    assert_eq!(u["isError"], false, "{u}");
    assert!(a.exists() && b.exists() && !a2.exists() && !b2.exists(), "both moves undone: {u}");
    let regs = c.rpc("kalango/registros", json!({}))["result"]["registros"].as_array().unwrap().clone();
    // only the moves were undone (the writes came before `desde`); reading the records added nothing
    assert!(regs.iter().filter(|e| e["tool"] == "arquivos.mover").all(|e| e["desfeito"] == true), "{regs:?}");
    assert!(regs.iter().filter(|e| e["tool"] == "arquivos.escrever").all(|e| e["desfeito"] == false), "{regs:?}");
    assert_eq!(u["structuredContent"]["desfeitos"].as_array().unwrap().len(), 2, "{u}");
    assert!(regs.len() > antes && regs.iter().filter(|e| e["tool"] == "kalango/registros").count() == 0);
}

#[test]
fn pastas_protegidas_e_fora_da_area() {
    let d = start();
    let mut c = Client::new(&d.sock);
    let segredo = d.area.join("segredo/chave");
    assert_eq!(c.call("arquivos.ler", json!({"caminho": segredo}))["isError"], true);
    // escapar com ..
    let escape = format!("{}/../../etc/hostname", d.area.display());
    assert_eq!(c.call("arquivos.ler", json!({"caminho": escape}))["isError"], true);
    // link simbólico dentro da área apontando para a pasta protegida
    std::os::unix::fs::symlink(d.area.join("segredo"), d.area.join("atalho")).unwrap();
    let r = c.call("arquivos.ler", json!({"caminho": d.area.join("atalho/chave")}));
    assert_eq!(r["isError"], true, "link para pasta protegida devia ser recusado: {r}");
    // listar a área não revela a pasta protegida
    let l = c.call("arquivos.listar", json!({"caminho": d.area}));
    let nomes: Vec<&str> = l["structuredContent"]["itens"].as_array().unwrap().iter().map(|i| i["nome"].as_str().unwrap()).collect();
    assert!(!nomes.contains(&"segredo"), "{nomes:?}");
}

/// The routes around a protected folder with the file tools, through the real kalangod: a link to a protected file
/// not created yet, and moving (or trashing) the folder that holds a protected one, to change it where it is not
/// protected and move it back.
#[test]
fn protected_folders_cannot_be_reached_around() {
    let d = start_cfg(|cfg| cfg.replace("/segredo\"]", "/cfg/kalango\"]"));
    let mut c = Client::new(&d.sock);
    let kalango = d.area.join("cfg/kalango");
    std::fs::create_dir_all(&kalango).unwrap();
    std::fs::write(kalango.join("agente-permissoes.json"), "{}").unwrap();
    std::fs::create_dir_all(d.area.join("docs")).unwrap();
    // a dangling link: writing through it would create cfg/kalango/conectores.json
    std::os::unix::fs::symlink(kalango.join("conectores.json"), d.area.join("docs/novo.json")).unwrap();
    let r = c.call("arquivos.escrever", json!({"caminho": d.area.join("docs/novo.json"), "conteudo": "x"}));
    assert_eq!(r["isError"], true, "{r}");
    assert!(!kalango.join("conectores.json").exists());
    // the folder holding it, moved away or to the trash, would carry it out of the protection
    let r = c.call("arquivos.mover", json!({"de": d.area.join("cfg"), "para": d.area.join("docs/cfg")}));
    assert_eq!(r["isError"], true, "{r}");
    let r = c.call("arquivos.lixeira", json!({"caminho": d.area.join("cfg")}));
    assert_eq!(r["isError"], true, "{r}");
    assert!(kalango.join("agente-permissoes.json").is_file());
    // nor can a folder made elsewhere be moved in to become it
    std::fs::create_dir_all(d.area.join("docs/outro/kalango")).unwrap();
    std::fs::remove_dir_all(d.area.join("cfg")).unwrap();
    let r = c.call("arquivos.mover", json!({"de": d.area.join("docs/outro"), "para": d.area.join("cfg")}));
    assert_eq!(r["isError"], true, "{r}");
    assert!(!d.area.join("cfg").exists());
    // the rest of the area works as before
    let r = c.call("arquivos.escrever", json!({"caminho": d.area.join("docs/notas.md"), "conteudo": "ok"}));
    assert_eq!(r["isError"], false, "{r}");
    let r = c.call("arquivos.mover", json!({"de": d.area.join("docs/notas.md"), "para": d.area.join("notas.md")}));
    assert_eq!(r["isError"], false, "{r}");
}

#[test]
fn trash_uses_the_configured_folder() {
    let d = start();
    let mut c = Client::new(&d.sock);
    let f = d.area.join("velho.txt");
    std::fs::write(&f, "rascunho").unwrap();
    let r = c.call("arquivos.lixeira", json!({"caminho": f}));
    assert_eq!(r["isError"], false, "{r}");
    let at = r["structuredContent"]["na_lixeira"].as_str().unwrap().to_string();
    let raiz = d.area.parent().unwrap().join("raiz/lixeira");
    assert!(at.starts_with(raiz.to_str().unwrap()), "the test trash, not a real one: {at}");
    assert!(!f.exists() && std::path::Path::new(&at).exists());
    assert!(raiz.join("info/velho.txt.trashinfo").exists());
}

#[test]
fn reads_pdf_text_with_page_markers() {
    let d = start();
    let mut c = Client::new(&d.sock);
    let pdf = d.area.join("relatorio-out.pdf");
    std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/relatorio-out.pdf"), &pdf).unwrap();
    let r = c.call("arquivos.ler", json!({"caminho": pdf}));
    assert_eq!(r["isError"], false, "{r}");
    let t = r["structuredContent"]["conteudo"].as_str().unwrap();
    assert_eq!(r["structuredContent"]["pdf_paginas"], 3, "{r}");
    let p2 = t.find("[p. 2]").expect("page 2 marker");
    let receita = t.find("Receita subiu 8%").expect("page 2 text");
    assert!(receita > p2 && t.find("[p. 3]").unwrap() > receita, "{t}");
    assert!(t.contains("20/11") && t.contains("despesas estáveis") && t.contains("Próxima revisão"), "accents must survive: {t}");
    // a broken PDF is an error, not a crash of the daemon
    std::fs::write(d.area.join("quebrado.pdf"), b"%PDF-1.4\nnot really a pdf").unwrap();
    assert_eq!(c.call("arquivos.ler", json!({"caminho": d.area.join("quebrado.pdf")}))["isError"], true);
    assert_eq!(c.call("arquivos.ler", json!({"caminho": pdf}))["isError"], false, "the daemon must still answer");
}

/// Approves the next request with "always" (as the approval screen's "Sempre" does).
fn approve_next_always(sock: PathBuf) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        for _ in 0..200 {
            let mut c = Client::new(&sock);
            let p = c.rpc("kalango/pendentes", json!({}));
            if let Some(id) = p["result"].as_array().and_then(|a| a.first()).and_then(|x| x["id"].as_u64()) {
                let out = Command::new(BIN).args(["aprovar", &id.to_string(), "--sempre"]).env("KALANGOD_SOCKET", &sock).output().unwrap();
                return String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        "nenhum pedido".into()
    })
}

#[test]
fn always_covers_only_the_same_request_and_can_be_taken_back() {
    let d = start();
    let mut c = Client::new(&d.sock);
    let args = json!({"nomes": ["cowsay"]});
    let h = approve_next_always(d.sock.clone());
    assert_eq!(c.call("pacotes.instalar", args.clone())["isError"], false);
    assert!(h.join().unwrap().contains("#"), "approved with always");
    // the same request again: no approval is asked (nobody approves here, and it still goes through)
    let t0 = std::time::Instant::now();
    let r = c.call("pacotes.instalar", args.clone());
    assert_eq!(r["isError"], false, "{r}");
    assert!(t0.elapsed() < Duration::from_secs(5));
    assert!(r["structuredContent"]["ponto_de_restauracao"].is_string(), "still a restore point first: {r}");
    // other arguments: asked again (approved here by the helper)
    let h = approve_next(d.sock.clone());
    assert_eq!(c.call("pacotes.instalar", json!({"nomes": ["sl"]}))["isError"], false);
    assert!(!h.join().unwrap().contains("nenhum pedido"), "a different request must ask");
    // listed, kept on disk, and taken back: then it asks again
    let l = c.rpc("kalango/sempre", json!({}));
    assert_eq!(l["result"]["respostas"].as_array().unwrap().len(), 1, "{l}");
    assert!(d.area.parent().unwrap().join("estado/aprovacoes-sempre.json").exists());
    let r = c.rpc("kalango/revogar_sempre", json!({"indice": 0}));
    assert!(r["result"]["revogada"]["tool"] == "pacotes.instalar", "{r}");
    let h = approve_next(d.sock.clone());
    assert_eq!(c.call("pacotes.instalar", args)["isError"], false);
    assert!(!h.join().unwrap().contains("nenhum pedido"), "after taking it back, it asks again");
    // full access is never "always"
    let h = approve_next_always(d.sock.clone());
    let r = c.call("sistema.executar", json!({"programa": "true", "argumentos": []}));
    let out = h.join().unwrap();
    assert!(out.contains("temporári") || out.contains("temporary"), "{out}");
    assert_eq!(r["isError"], true, "not approved: {r}");
}

#[test]
fn niveis_sistema_e_total() {
    let d = start();
    let mut c = Client::new(&d.sock);
    // nível sistema: aprovação + ponto de restauração; em simulação, nada é instalado
    let h = approve_next(d.sock.clone());
    let r = c.call("pacotes.instalar", json!({"nomes": ["cowsay"]}));
    h.join().unwrap();
    assert_eq!(r["isError"], false, "{r}");
    assert!(r["structuredContent"]["saida"].as_str().unwrap().contains("[simulado]"));
    assert!(r["structuredContent"]["ponto_de_restauracao"].is_string());
    assert_eq!(r["structuredContent"]["desfazivel"], true);
    // nomes que parecem opções são recusados
    let h = approve_next(d.sock.clone());
    let bad = c.call("pacotes.instalar", json!({"nomes": ["--allow-unauthenticated"]}));
    h.join().unwrap();
    assert_eq!(bad["isError"], true);

    // nível total: a primeira chamada pede aprovação e abre a concessão; a segunda não pede
    let h = approve_next(d.sock.clone());
    let t1 = c.call("sistema.executar", json!({"programa": "id"}));
    h.join().unwrap();
    assert_eq!(t1["isError"], false, "{t1}");
    let quem = c.rpc("kalango/quem", json!({}));
    assert!(quem["result"]["nivel_total_restante_s"].as_u64().unwrap() > 800);
    let t2 = c.call("sistema.executar", json!({"programa": "id"}));
    assert_eq!(t2["isError"], false);
}

#[test]
fn negar_e_autoaprovacao() {
    let d = start();
    let mut c = Client::new(&d.sock);
    let sock = d.sock.clone();
    let h = std::thread::spawn(move || {
        for _ in 0..200 {
            let mut c2 = Client::new(&sock);
            if let Some(id) = c2.rpc("kalango/pendentes", json!({}))["result"].as_array().and_then(|a| a.first()).and_then(|x| x["id"].as_u64()) {
                // o mesmo processo do pedido não pode aprovar
                let own = c2.rpc("kalango/decidir", json!({"id": id, "aprovar": true}));
                let out = Command::new(BIN).args(["negar", &id.to_string()]).env("KALANGOD_SOCKET", &sock).output().unwrap();
                return (own, String::from_utf8_lossy(&out.stdout).into_owned());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        (Value::Null, String::new())
    });
    let r = c.call("servicos.acao", json!({"nome": "cron.service", "acao": "parar"}));
    let (own, neg) = h.join().unwrap();
    assert!(own["error"]["message"].as_str().unwrap_or("").contains("próprio pedido"), "{own}");
    assert!(neg.contains("negado"));
    assert_eq!(r["isError"], true);
}

// ---------------------------------------------------------------------------------------------------------------
// Tarefas com limites (cgroup próprio) e Landlock. Sem root, o limite vem do systemd do usuário.

fn has_user_systemd() -> bool {
    Command::new("systemd-run").args(["--user", "--scope", "--quiet", "true"]).status().map(|s| s.success()).unwrap_or(false)
}

#[test]
fn tarefas_com_limites() {
    if !has_user_systemd() {
        eprintln!("sem systemd do usuário: teste pulado");
        return;
    }
    let d = start_with(false, "[limites]\nmemoria_mb = 256\ncpu_pct = 100\nprocessos = 64\ntempo_s = 30");
    let mut c = Client::new(&d.sock);
    // normal: roda, devolve a saída e o código
    let r = c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", "echo olá; exit 3"], "pasta": d.area}));
    assert_eq!(r["isError"], false, "{r}");
    let v = &r["structuredContent"];
    assert_eq!(v["saida"], "olá\n", "{v}");
    assert_eq!(v["codigo"], 3);
    // memória: pede 400 MB com limite de 64 MB → morto pelo kernel (OOM dentro do cgroup da tarefa)
    let r = c.call("tarefas.executar", json!({"programa": "python3", "argumentos": ["-c", "x = bytearray(400*1024*1024); print('não devia chegar aqui')"], "memoria_mb": 64}));
    let v = &r["structuredContent"];
    assert_eq!(v["estourou_memoria"], true, "{v}");
    assert_eq!(v["sinal"], 9, "{v}");
    assert!(!v["saida"].as_str().unwrap().contains("chegar"));
    // o agente não consegue pedir mais que a configuração
    assert_eq!(v["limites"]["memoria_mb"], 64);
    let r = c.call("tarefas.executar", json!({"programa": "true", "memoria_mb": 999999}));
    assert_eq!(r["structuredContent"]["limites"]["memoria_mb"], 256);
    // tempo: encerrada no limite
    let r = c.call("tarefas.executar", json!({"programa": "sleep", "argumentos": ["30"], "tempo_s": 1}));
    let v = &r["structuredContent"];
    assert_eq!(v["estourou_tempo"], true, "{v}");
    assert!(v["tempo_ms"].as_u64().unwrap() < 5000, "{v}");
    // processos: com limite de 8, criar 40 falha
    let r = c.call("tarefas.executar", json!({"programa": "python3", "argumentos": ["-c", "import os,time\nn=0\nfor i in range(40):\n  try:\n    if os.fork()==0:\n      time.sleep(2); os._exit(0)\n    n+=1\n  except OSError: pass\nprint(n)"], "processos": 8}));
    let n: u32 = r["structuredContent"]["saida"].as_str().unwrap().trim().parse().unwrap_or(99);
    assert!(n < 8, "devia ter criado menos de 8 processos, criou {n}: {r}");
    // Landlock: escreve na área permitida, não fora dela (mesmo onde o usuário poderia)
    let home = std::env::var("HOME").unwrap();
    let fora = format!("{home}/.kalango-teste-landlock-{}", std::process::id());
    let r = c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", format!("echo a > {}/dentro.txt && echo b > {fora}", d.area.display())]}));
    let v = &r["structuredContent"];
    assert!(d.area.join("dentro.txt").exists(), "{v}");
    assert!(!Path::new(&fora).exists(), "Landlock devia impedir escrever em {fora}: {v}");
    assert!(v["erros"].as_str().unwrap().contains("Permission denied"), "{v}");
    // pasta fora da área é recusada antes de rodar
    let r = c.call("tarefas.executar", json!({"programa": "true", "pasta": "/etc"}));
    assert_eq!(r["isError"], true);
}

// ---------------------------------------------------------------------------------------------------------------
// Pontos de restauração reais (cópia do /etc numa raiz de teste): criar, mudar, restaurar e desfazer a restauração.

#[test]
fn pontos_de_restauracao() {
    let d = start_with(false, "pontos_max = 3");
    let etc = d.raiz.join("etc");
    std::fs::create_dir_all(etc.join("rede")).unwrap();
    std::fs::write(etc.join("hostname"), "kalango\n").unwrap();
    std::fs::write(etc.join("rede/config"), "dhcp=sim\n").unwrap();
    std::os::unix::fs::symlink("hostname", etc.join("atalho")).unwrap();
    let mut c = Client::new(&d.sock);

    let h = approve_next(d.sock.clone());
    let p = c.call("pontos.criar", json!({"descricao": "antes do teste"}));
    h.join().unwrap();
    assert_eq!(p["isError"], false, "{p}");
    let id = p["structuredContent"]["ponto"]["id"].as_str().unwrap().to_string();
    assert_eq!(p["structuredContent"]["ponto"]["tipo"], "arquivos");

    // muda tudo: troca, cria, apaga
    std::fs::write(etc.join("hostname"), "outro\n").unwrap();
    std::fs::write(etc.join("novo.conf"), "x").unwrap();
    std::fs::remove_dir_all(etc.join("rede")).unwrap();

    // o plano mostra exatamente o que vai mudar
    let h = approve_next(d.sock.clone());
    let plano = c.call("pontos.restaurar", json!({"id": id, "so_plano": true}));
    h.join().unwrap();
    let pl = &plano["structuredContent"]["plano"]["etc"];
    assert_eq!(pl["apagar"], json!(["novo.conf"]), "{plano}");
    assert_eq!(pl["trocar"], json!(["hostname"]));
    assert!(pl["recriar"].as_array().unwrap().contains(&json!("rede/config")));
    assert!(etc.join("novo.conf").exists(), "só plano não muda nada");

    let h = approve_next(d.sock.clone());
    let r = c.call("pontos.restaurar", json!({"id": id}));
    h.join().unwrap();
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(std::fs::read_to_string(etc.join("hostname")).unwrap(), "kalango\n");
    assert_eq!(std::fs::read_to_string(etc.join("rede/config")).unwrap(), "dhcp=sim\n");
    assert!(!etc.join("novo.conf").exists());
    assert_eq!(std::fs::read_link(etc.join("atalho")).unwrap(), Path::new("hostname"));

    // a restauração também se desfaz (volta ao estado de logo antes dela)
    let reg = r["structuredContent"]["registro"].as_u64().unwrap();
    let h = approve_next(d.sock.clone());
    let u = c.call("linha_do_tempo.desfazer", json!({"id": reg}));
    h.join().unwrap();
    assert_eq!(u["isError"], false, "{u}");
    assert_eq!(std::fs::read_to_string(etc.join("hostname")).unwrap(), "outro\n");
    assert!(etc.join("novo.conf").exists());
    assert!(!etc.join("rede").exists());

    // toda ação de nível sistema cria o seu ponto antes; a retenção mantém só 3
    let l = c.call("pontos.listar", json!({}));
    let n = l["structuredContent"]["pontos"].as_array().unwrap().len();
    assert!(n <= 3 && n >= 2, "{l}");
}

// ---------------------------------------------------------------------------------------------------------------
// Ferramentas de administração: as de leitura rodam de verdade; as de mudança validam entradas e respeitam a simulação.

#[test]
fn administracao() {
    let d = start();
    let mut c = Client::new(&d.sock);
    let names: Vec<String> = c.rpc("tools/list", json!({}))["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    for t in ["rede.conexoes", "rede.wifi", "rede.conexao", "firewall.estado", "firewall.regra", "firewall.ligar", "discos.dispositivos", "discos.saude",
        "discos.montar", "discos.desmontar", "usuarios.listar", "usuarios.criar", "usuarios.remover", "usuarios.grupo", "atualizacoes.listar",
        "atualizacoes.verificar", "atualizacoes.aplicar", "pacotes.versoes", "tarefas.executar", "pontos.listar", "pontos.criar", "pontos.restaurar"] {
        assert!(names.contains(&t.to_string()), "falta {t}");
    }
    let u = c.call("usuarios.listar", json!({}));
    assert_eq!(u["isError"], false, "{u}");
    let me = std::env::var("USER").unwrap_or_default();
    assert!(u["structuredContent"]["usuarios"].as_array().unwrap().iter().any(|x| x["nome"] == me.as_str()), "{u}");
    let dsk = c.call("discos.dispositivos", json!({}));
    assert_eq!(dsk["isError"], false, "{dsk}");
    assert!(!dsk["structuredContent"]["blockdevices"].as_array().unwrap().is_empty());
    let n = c.call("rede.conexoes", json!({}));
    assert_eq!(n["isError"], false, "{n}");
    let a = c.call("atualizacoes.listar", json!({}));
    assert_eq!(a["isError"], false, "{a}");

    // entradas perigosas são recusadas antes de qualquer aprovação ou execução
    for (tool, args) in [
        ("discos.saude", json!({"dispositivo": "/dev/../etc/shadow"})),
        ("discos.saude", json!({"dispositivo": "/etc/passwd"})),
    ] {
        let r = c.call(tool, args.clone());
        assert_eq!(r["isError"], true, "{tool} {args}");
    }
    for (tool, args) in [
        ("usuarios.criar", json!({"nome": "--badname"})),
        ("usuarios.criar", json!({"nome": "Root;rm"})),
        ("usuarios.remover", json!({"nome": "root"})),
        ("usuarios.remover", json!({"nome": me})),
        ("firewall.regra", json!({"acao": "permitir", "porta": "22; reboot"})),
        ("pacotes.versoes", json!({"versoes": ["bash=--force"]})),
    ] {
        let h = approve_next(d.sock.clone());
        let r = c.call(tool, args.clone());
        h.join().unwrap();
        assert_eq!(r["isError"], true, "{tool} {args} devia ser recusado: {r}");
    }
    // em simulação, mudanças válidas descrevem o comando e trazem o inverso
    let h = approve_next(d.sock.clone());
    let r = c.call("firewall.regra", json!({"acao": "permitir", "porta": "8080/tcp"}));
    h.join().unwrap();
    assert!(r["structuredContent"]["saida"].as_str().unwrap().contains("[simulado] ufw allow 8080/tcp"), "{r}");
    assert_eq!(r["structuredContent"]["desfazivel"], true);
    let h = approve_next(d.sock.clone());
    let r = c.call("usuarios.criar", json!({"nome": "kalango-teste", "nome_completo": "Teste"}));
    h.join().unwrap();
    assert!(r["structuredContent"]["saida"].as_str().unwrap().contains("[simulado] useradd -m -s /bin/bash -c Teste kalango-teste"), "{r}");
}

// ---------------------------------------------------------------------------------------------------------------

#[test]
fn concessao_total_expira() {
    let d = start_cfg(|c| c.replace("total_grant_s = 900", "total_grant_s = 1"));
    let mut c = Client::new(&d.sock);
    let h = approve_next(d.sock.clone());
    assert_eq!(c.call("sistema.executar", json!({"programa": "true"}))["isError"], false);
    h.join().unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    assert!(c.rpc("kalango/quem", json!({}))["result"]["nivel_total_restante_s"].is_null(), "concessão devia ter expirado");
    // depois do prazo, pede aprovação de novo: sem ninguém aprovar, a chamada espera e é negada pelo prazo curto
    let neg = std::thread::spawn({
        let sock = d.sock.clone();
        move || {
            for _ in 0..200 {
                let mut c2 = Client::new(&sock);
                if let Some(id) = c2.rpc("kalango/pendentes", json!({}))["result"].as_array().and_then(|a| a.first()).and_then(|x| x["id"].as_u64()) {
                    return Command::new(BIN).args(["negar", &id.to_string()]).env("KALANGOD_SOCKET", &sock).output().is_ok();
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            false
        }
    });
    let r = c.call("sistema.executar", json!({"programa": "true"}));
    assert!(neg.join().unwrap(), "devia ter aparecido um pedido de aprovação novo");
    assert_eq!(r["isError"], true);
}

#[test]
fn chamador_sem_regra_e_recusado() {
    // só o root tem regra: este processo (não root) é recusado mesmo dizendo ser outro
    let d = start_cfg(|c| c.replace("name = \"teste\"", "name = \"so-root\"\nuid = 0"));
    let mut c = Client::new(&d.sock);
    let r = c.rpc("initialize", json!({"protocolVersion": "2025-06-18", "clientInfo": {"name": "root", "uid": 0}}));
    assert!(r["error"]["message"].as_str().unwrap().contains("sem regra"), "{r}");
    let r = c.rpc("tools/list", json!({}));
    assert!(r["error"].is_object(), "{r}");
}

// ---------------------------------------------------------------------------------------------------------------
// Tela de aprovação: só um executável confiável aprova sem terminal; quem espera é avisado na hora.

/// Copia o kalangod para uma pasta 0755 (como /usr/libexec/kalango) para servir de "aprovador gráfico".
fn trusted_copy(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let d = dir.join("libexec");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
    let exe = d.join("kalango-aprovacao");
    std::fs::copy(BIN, &exe).unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    exe
}

/// Aprova pelo executável dado, sem terminal de controle (setsid), e devolve a mensagem.
fn approve_with(exe: &Path, sock: &Path) -> String {
    let mut c = Client::new(sock);
    let (_, list) = loop {
        let r = c.rpc("kalango/aguardar", json!({"max_s": 5}))["result"].clone();
        if !r["pendentes"].as_array().unwrap().is_empty() {
            break (r["geracao"].clone(), r["pendentes"].clone());
        }
    };
    let id = list[0]["id"].as_u64().unwrap();
    let out = Command::new("setsid").arg("-w").arg(exe).args(["aprovar", &id.to_string()]).env("KALANGOD_SOCKET", sock)
        .stdin(std::process::Stdio::null()).output().unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
}

#[test]
fn aprovador_grafico_confiavel() {
    let tmp = tempfile::tempdir().unwrap();
    let trusted = trusted_copy(tmp.path());
    let t = trusted.display().to_string();
    let d = start_cfg(move |c| c.replace("approval_requires_tty = false", &format!("approval_requires_tty = true\naprovadores_graficos = [\"{t}\"]")));
    let mut c = Client::new(&d.sock);

    // sem terminal e não confiável: recusado
    let sock = d.sock.clone();
    let h = std::thread::spawn(move || approve_with(Path::new(BIN), &sock));
    let sock2 = d.sock.clone();
    let r = std::thread::spawn(move || Client::new(&sock2).call("pacotes.instalar", json!({"nomes": ["cowsay"]})));
    let msg = h.join().unwrap();
    assert!(msg.contains("presencial"), "executável não confiável devia ser recusado: {msg}");
    // o pedido continua esperando: agora o aprovador confiável decide
    let msg = approve_with(&trusted, &d.sock);
    assert!(msg.contains("aprovado"), "{msg}");
    let r = r.join().unwrap();
    assert_eq!(r["isError"], false, "{r}");

    // a lista entregue à tela traz o que ela precisa mostrar
    // (waits from the current generation: earlier requests in this daemon already moved it past 0)
    let geracao = c.rpc("kalango/aguardar", json!({}))["result"]["geracao"].as_u64().unwrap();
    let sock = d.sock.clone();
    let w = std::thread::spawn(move || Client::new(&sock).rpc("kalango/aguardar", json!({"geracao": geracao, "max_s": 10}))["result"].clone());
    let h = std::thread::spawn({ let s = d.sock.clone(); let t = trusted.clone(); move || approve_with(&t, &s) });
    let _ = c.call("servicos.acao", json!({"nome": "cron.service", "acao": "parar"}));
    h.join().unwrap();
    let got = w.join().unwrap();
    let p = &got["pendentes"][0];
    for k in ["descricao", "efeito", "reversivel", "prazo_s", "caller", "args", "level"] {
        assert!(!p[k].is_null(), "falta {k}: {got}");
    }
    assert_eq!(p["efeito"], "escreve");
    assert_eq!(p["reversivel"], true);

    // um executável confiável mas que outros podem alterar deixa de ser confiável
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&trusted, std::fs::Permissions::from_mode(0o777)).unwrap();
    let sock = d.sock.clone();
    let r = std::thread::spawn(move || Client::new(&sock).call("pacotes.instalar", json!({"nomes": ["cowsay"]})));
    let msg = approve_with(&trusted, &d.sock);
    assert!(msg.contains("presencial"), "arquivo gravável por todos não pode aprovar: {msg}");
    std::fs::set_permissions(&trusted, std::fs::Permissions::from_mode(0o755)).unwrap();
    approve_with(&trusted, &d.sock);
    r.join().unwrap();
}

/// Settings › Network › "Permitir agentes usarem esta rede": on a network the person keeps the agents off, `web.ler`
/// refuses (and says which network); allowed again, the network no longer stops it.
#[test]
fn rede_sem_agentes_bloqueia_o_web_ler() {
    let dir = tempfile::tempdir().unwrap();
    let nmcli = dir.path().join("nmcli");
    std::fs::write(&nmcli, "#!/bin/sh\necho 'Celular de Ana'\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&nmcli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let d = start_full_env(true, "", |c| c, &[("KALANGO_NMCLI", nmcli.to_str().unwrap())]);
    let mut c = Client::new(&d.sock);
    assert_eq!(c.rpc("kalango/redes_agentes", json!({}))["result"]["sem_agentes"], json!([]));
    let r = c.rpc("kalango/redes_agentes", json!({"rede": "Celular de Ana", "permitir": false}));
    assert_eq!(r["result"]["sem_agentes"], json!(["Celular de Ana"]));
    let w = c.call("web.ler", json!({"url": "https://exemplo.invalid/"}));
    assert_eq!(w["isError"], true, "{w}");
    assert!(w["content"][0]["text"].as_str().unwrap().contains("Celular de Ana"), "{w}");
    c.rpc("kalango/redes_agentes", json!({"rede": "Celular de Ana", "permitir": true}));
    let w = c.call("web.ler", json!({"url": "https://exemplo.invalid/"}));
    // (the address does not exist: it still fails, but not because of the network)
    assert!(!w["content"][0]["text"].as_str().unwrap_or("").contains("Celular de Ana"), "{w}");
}
