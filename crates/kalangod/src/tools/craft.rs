//! Os Apps criativos (projeto Craft) como ferramentas `craft.*` dos agentes.
//!
//! O kalangod é o único caminho: ele mesmo inicia o `<app>-cli mcp` de cada app, como a pessoa que chamou (o uid dela),
//! dentro do Flatpak do app (`flatpak run --command=<cli>`, sem rede, sem o serviço do Flatpak no computador e, sem
//! janela, sem tela nem som), e guarda a sessão (o documento do servidor vive entre as chamadas; uma sessão parada por
//! 10 min é encerrada). Com `janela`, o kalangod abre a janela do app e o servidor no mesmo sandbox, com uma ficha de
//! controle que ele gera a cada vez e entrega só aos dois processos (nunca num arquivo).
//!
//! Cada chamada passa pelos níveis do catálogo (`kalango_craft::niveis`): ler, inspecionar, listar e ver prévia são
//! livres; salvar, exportar, importar e caminhos de arquivo pedem confirmação; controlar a janela, scripts, comandos não
//! conferidos e ferramentas desconhecidas pedem aprovação (sem "sempre"). Tudo entra na linha do tempo.
//! Se os agentes podem usar cada app é da pessoa (Central de Apps › app › "Agentes podem usar este app"), guardado aqui
//! no estado do kalangod (`craft/<uid>.json`), onde os agentes não alcançam.

use super::{n_, obj, s, tr, trf, Ctx, Out, Tool};
use crate::peer::Peer;
use crate::policy::{Effect, Level};
use kalango_craft::niveis::{classificar, Nivel};
use kalango_craft::{catalogo, App};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

pub fn tools() -> Vec<Tool> {
    use Effect::*;
    use Level::*;
    let t =
        |name, description, level, effect, reversible, schema: fn() -> Value, run| Tool { name, description, level, effect, reversible, schema, run };
    vec![
        t("craft.apps", n_("The creative apps (Craft project) installed for you, whether agents may use each, and — with ferramentas — each app's MCP tools with the level KalangoOS applies (free, confirm, approve)"), Observar, Leitura, false,
            || obj(json!({"app": {"type": "string"}, "ferramentas": {"type": "boolean"}}), &[]), apps),
        t("craft.chamar", n_("Calls one MCP tool of an installed creative app (headless, or on a window of the app KalangoOS opens with janela). Reading, inspecting, listing and previews are free; saving, exporting, importing and file paths ask to confirm; driving the window, scripts and unknown tools ask for approval"), Usuario, Escreve, false,
            || obj(json!({"app": {"type": "string", "description": "id, nome ou imagem (filmcraft)"}, "ferramenta": {"type": "string"}, "argumentos": {"type": "object"}, "janela": {"type": "boolean"},
                "por_que": {"type": "string", "description": "o pedido da pessoa que leva a esta chamada, em poucas palavras (aparece na tela de aprovação)"}}), &["app", "ferramenta"]), chamar),
        t("craft.encerrar", n_("Ends the session with a creative app's server (what was not saved is lost)"), Usuario, Escreve, false,
            || obj(json!({"app": {"type": "string"}, "janela": {"type": "boolean"}}), &["app"]), encerrar),
    ]
}

// ------------------------------------------------------------------------------------------------ the person's choice

fn ajustes_path(state_dir: &Path, uid: u32) -> PathBuf {
    state_dir.join("craft").join(format!("{uid}.json"))
}

/// Se os agentes podem usar este app (ligado por padrão: os níveis valem sempre).
pub fn agentes_podem(state_dir: &Path, uid: u32, id: &str) -> bool {
    ler_ajustes(state_dir, uid)[id]["agentes"].as_bool().unwrap_or(true)
}

/// As escolhas da pessoa. Sem o arquivo: nada escolhido (os padrões valem). Com o arquivo ilegível, vazio ou que não é um
/// objeto JSON (cortado por uma queda de luz, por exemplo): falha fechado, com os agentes desligados em todos os apps
/// até a pessoa escolher de novo; nunca volta aos padrões ligados.
fn ler_ajustes(state_dir: &Path, uid: u32) -> Value {
    match std::fs::read_to_string(ajustes_path(state_dir, uid)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Ok(t) if serde_json::from_str::<Value>(&t).is_ok_and(|v| v.is_object()) => serde_json::from_str(&t).unwrap_or(json!({})),
        _ => Value::Object(catalogo().apps.iter().map(|a| (a.id.clone(), json!({"agentes": false}))).collect()),
    }
}

/// O que o shell pede (`kalango/craft`, só da interface da pessoa): ler, ou mudar se os agentes podem usar um app e as
/// pastas que o servidor dele vê. As pastas ficam aqui, no estado do kalangod (nunca num arquivo da casa da pessoa, que
/// os programas dela, os agentes inclusive, podem escrever); cada uma é conferida ao entrar e de novo ao usar.
pub fn rpc(cfg: &crate::policy::Config, peer: &Peer, params: &Value) -> Result<Value, String> {
    let p = ajustes_path(&cfg.state_dir, peer.uid);
    let ler_ajustes = || ler_ajustes(&cfg.state_dir, peer.uid);
    match params["acao"].as_str().unwrap_or("") {
        "ler" => Ok(ler_ajustes()),
        "definir" => {
            let id = params["app"].as_str().ok_or("app?")?;
            let app = catalogo().app(id).ok_or_else(|| tr("not a creative app of the catalog").to_string())?;
            let id = app.id.clone();
            // (ler, mudar e gravar de uma vez só, sob o cadeado: outra chamada ao mesmo tempo não desfaz esta)
            let v = {
                let _g = ESTADO.lock().unwrap_or_else(|e| e.into_inner());
                let mut v = ler_ajustes();
                if !v[&id].is_object() {
                    v[&id] = json!({});
                }
                if let Some(on) = params["agentes"].as_bool() {
                    v[&id]["agentes"] = json!(on);
                }
                if let Some(on) = params["rede"].as_bool() {
                    v[&id]["rede"] = json!(on);
                }
                if let Some(ps) = params["pastas"].as_array() {
                    let nomes: Vec<String> = ps.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
                    for n in &nomes {
                        pasta_segura(cfg, peer, n)?;
                    }
                    v[&id]["pastas"] = json!(nomes);
                }
                gravar_atomico(&p, &v.to_string())?;
                v
            };
            // (desligar "Agentes podem usar este app" age já: a sessão aberta é morta, mesmo com uma chamada em curso)
            if params["agentes"].as_bool() == Some(false) || params["pastas"].is_array() {
                fechar_do_app(peer.uid, &id);
            }
            // (a escolha vale já para o app instalado: as permissões do Flatpak são refeitas daqui)
            if (params.get("rede").is_some() || params.get("pastas").is_some()) && instalados(peer).contains(&id) {
                aplicar_permissoes(cfg, peer, app)?;
            }
            Ok(v)
        }
        // as pastas conferidas de um app (o que a Central mostra), e refazer as permissões dele (depois de instalar)
        "pastas" => {
            let app = catalogo().app(params["app"].as_str().ok_or("app?")?).ok_or_else(|| tr("not a creative app of the catalog").to_string())?;
            let caminhos = pastas(cfg, peer, app)?;
            Ok(json!({"nomes": nomes_das_pastas(cfg, peer.uid, app), "caminhos": caminhos, "rede": rede(cfg, peer.uid, &app.id)}))
        }
        // verificar, baixar e importar a versão atual no repositório protegido (demora: o shell espera)
        "preparar" => {
            let app = catalogo().app(params["app"].as_str().ok_or("app?")?).ok_or_else(|| tr("not a creative app of the catalog").to_string())?;
            // (um preparo por pessoa e app: o segundo pedido não pisa no download do primeiro)
            let Some(curso) = PreparoEmCurso::tentar(peer.uid, &app.id) else {
                return Ok(json!({"erro": {"tipo": "em_andamento", "texto": tr("this app is already being prepared")}}));
            };
            let instalada = verificados(cfg, peer.uid).get(&app.id).and_then(|e| e.atual.as_ref().map(|x| x.1.clone()));
            let a = curso.andamento.clone();
            let repo = repositorio(cfg);
            let r = kalango_craft::instalar::preparar(catalogo(), app, &repo, &pasta_de_download(cfg, peer.uid), instalada.as_deref(), &a);
            match r {
                Ok((commit, versao, digest)) => {
                    registrar_preparado(cfg, peer.uid, &repo, &app.id, &commit, &versao, &digest)?;
                    Ok(json!({"commit": commit, "versao": versao, "digest": digest, "repo": repo}))
                }
                Err(e) => Ok(json!({"erro": erro_json(&e)})),
            }
        }
        "cancelar" => {
            let id = catalogo().app(params["app"].as_str().unwrap_or("")).map(|a| a.id.clone()).unwrap_or_default();
            if let Some(a) = ANDAMENTO.lock().unwrap_or_else(|e| e.into_inner()).get(&(peer.uid, id)) {
                a.cancelar.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            Ok(json!({}))
        }
        "andamento" => {
            let id = catalogo().app(params["app"].as_str().unwrap_or("")).map(|a| a.id.clone()).unwrap_or_default();
            let g = ANDAMENTO.lock().unwrap_or_else(|e| e.into_inner());
            Ok(match g.get(&(peer.uid, id)) {
                Some(a) => {
                    json!({"feito": a.feito.load(std::sync::atomic::Ordering::Relaxed), "total": a.total.load(std::sync::atomic::Ordering::Relaxed)})
                }
                None => json!({}),
            })
        }
        // o shell instalou um dos preparados: ele passa a ser o atual, e o atual de antes vira ponto de restauração
        "instalado" => {
            let app = catalogo().app(params["app"].as_str().ok_or("app?")?).ok_or_else(|| tr("not a creative app of the catalog").to_string())?;
            let commit = params["commit"].as_str().ok_or("commit?")?;
            let (pontos, preparados) = alterar_verificados(cfg, peer.uid, |est| {
                let e = est.entry(app.id.clone()).or_default();
                let pos =
                    e.preparados.iter().position(|p| p.0 == commit).ok_or_else(|| tr("this commit was not verified by KalangoOS").to_string())?;
                let p = e.preparados.remove(pos);
                if let Some(atual) = e.atual.take().filter(|a| a.0 != p.0) {
                    e.anteriores.insert(0, atual);
                    e.anteriores.truncate(kalango_craft::instalar::PONTOS);
                }
                e.atual = Some((p.0, p.1));
                Ok((e.anteriores.clone(), e.preparados.clone()))
            })?;
            let repo = repositorio(cfg);
            kalango_craft::instalar::guardar_pontos(&repo, peer.uid, &app.id, &pontos)?;
            kalango_craft::instalar::guardar_preparados(&repo, peer.uid, &app.id, &preparados)?;
            Ok(json!({"pontos": pontos}))
        }
        // o ponto de restauração mais novo (só um commit que o kalangod verificou); e, depois de voltar, a lista anda
        "anterior" => {
            let app = catalogo().app(params["app"].as_str().ok_or("app?")?).ok_or_else(|| tr("not a creative app of the catalog").to_string())?;
            let est = verificados(cfg, peer.uid);
            let e = est.get(&app.id).cloned().unwrap_or_default();
            let (c, v) = e.anteriores.first().cloned().ok_or_else(|| tr("no restore point").to_string())?;
            Ok(json!({"commit": c, "versao": v, "repo": repositorio(cfg)}))
        }
        "voltou" => {
            let app = catalogo().app(params["app"].as_str().ok_or("app?")?).ok_or_else(|| tr("not a creative app of the catalog").to_string())?;
            let commit = params["commit"].as_str().ok_or("commit?")?;
            let pontos = alterar_verificados(cfg, peer.uid, |est| {
                let e = est.entry(app.id.clone()).or_default();
                if e.anteriores.first().map(|x| x.0.as_str()) != Some(commit) {
                    return Err(tr("this commit was not verified by KalangoOS").into());
                }
                let volta = e.anteriores.remove(0);
                e.atual = Some(volta);
                Ok(e.anteriores.clone())
            })?;
            kalango_craft::instalar::guardar_pontos(&repositorio(cfg), peer.uid, &app.id, &pontos)?;
            Ok(json!({"pontos": pontos}))
        }
        "aplicar" => {
            let app = catalogo().app(params["app"].as_str().ok_or("app?")?).ok_or_else(|| tr("not a creative app of the catalog").to_string())?;
            aplicar_permissoes(cfg, peer, app)?;
            Ok(json!({"aplicado": app.id}))
        }
        _ => Err(tr("unknown action").into()),
    }
}

/// O repositório dos apps verificados: só o kalangod escreve, todos leem (o Flatpak da pessoa instala dele).
pub fn repositorio(cfg: &crate::policy::Config) -> PathBuf {
    if nix::unistd::geteuid().is_root() {
        PathBuf::from("/var/lib/kalango-apps-criativos/repo")
    } else {
        cfg.state_dir.join("craft/repo")
    }
}

/// O andamento dos preparos em curso: (uid, app) → bytes. Estar aqui é também o cadeado do preparo: um por pessoa e app.
static ANDAMENTO: std::sync::LazyLock<Mutex<HashMap<(u32, String), kalango_craft::verificar::Andamento>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// O preparo de um app de uma pessoa, enquanto dura (sai do mapa ao ser solto, até num pânico).
struct PreparoEmCurso {
    chave: (u32, String),
    andamento: kalango_craft::verificar::Andamento,
}

impl PreparoEmCurso {
    /// `None` se esta pessoa já está preparando este app.
    fn tentar(uid: u32, id: &str) -> Option<PreparoEmCurso> {
        let mut g = ANDAMENTO.lock().unwrap_or_else(|e| e.into_inner());
        let chave = (uid, id.to_string());
        if g.contains_key(&chave) {
            return None;
        }
        let a = kalango_craft::verificar::Andamento::default();
        g.insert(chave.clone(), a.clone());
        Some(PreparoEmCurso { chave, andamento: a })
    }
}

impl Drop for PreparoEmCurso {
    fn drop(&mut self) {
        ANDAMENTO.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.chave);
    }
}

/// Onde o preparo de uma pessoa baixa: uma pasta por pessoa (dois preparos do mesmo app de pessoas diferentes não se
/// pisam).
fn pasta_de_download(cfg: &crate::policy::Config, uid: u32) -> PathBuf {
    cfg.state_dir.join("craft/baixando").join(uid.to_string())
}

/// Os cadeados do estado do kalangod (a lista dos verificados e as escolhas da pessoa): ler, mudar e gravar de uma vez,
/// nunca ler antes de uma espera longa e gravar depois.
static ESTADO: Mutex<()> = Mutex::new(());

fn verificados(cfg: &crate::policy::Config, uid: u32) -> std::collections::BTreeMap<String, kalango_craft::instalar::Verificado> {
    std::fs::read_to_string(cfg.state_dir.join("craft").join(format!("{uid}-verificados.json")))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Troca o arquivo de uma vez: grava num arquivo ao lado (nome só deste momento) e renomeia por cima.
fn gravar_atomico(p: &Path, texto: &str) -> Result<(), String> {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = p.with_extension(format!("tmp{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    // (o conteúdo no disco antes de trocar o nome, e a pasta no disco depois: uma queda de luz deixa o arquivo velho ou o novo,
    // nunca um arquivo cortado. O fsync não tem teste automático: só uma queda de energia o distingue de um write comum;
    // ele está aqui por revisão, e os testes cobrem o que dá para ver: troca inteira, nada sobrando, erro limpo)
    let gravar = || -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(texto.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, p)?;
        if let Some(d) = p.parent() {
            std::fs::File::open(d)?.sync_all()?;
        }
        Ok(())
    };
    gravar().map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

/// Lê os verificados, deixa `f` mudar e grava, tudo sob o cadeado: o que outra chamada gravou no meio tempo não se perde.
fn alterar_verificados<R>(
    cfg: &crate::policy::Config,
    uid: u32,
    f: impl FnOnce(&mut std::collections::BTreeMap<String, kalango_craft::instalar::Verificado>) -> Result<R, String>,
) -> Result<R, String> {
    let _g = ESTADO.lock().unwrap_or_else(|e| e.into_inner());
    let mut v = verificados(cfg, uid);
    let r = f(&mut v)?;
    gravar_atomico(&cfg.state_dir.join("craft").join(format!("{uid}-verificados.json")), &serde_json::to_string(&v).unwrap_or_default())?;
    Ok(r)
}

/// O preparo terminou: o commit entra nos preparados (relido agora, não o de antes do download) e ganha uma referência no
/// repositório.
fn registrar_preparado(cfg: &crate::policy::Config, uid: u32, repo: &Path, id: &str, commit: &str, versao: &str, digest: &str) -> Result<(), String> {
    let lista = alterar_verificados(cfg, uid, |est| {
        let e = est.entry(id.to_string()).or_default();
        e.preparados.retain(|p| p.0 != commit);
        e.preparados.insert(0, (commit.to_string(), versao.to_string(), digest.to_string()));
        e.preparados.truncate(kalango_craft::instalar::PREPARADOS);
        Ok(e.preparados.clone())
    })?;
    kalango_craft::instalar::guardar_preparados(repo, uid, id, &lista)
}

/// Um erro de preparo como a tela o mostra (o design tem um estado para cada).
fn erro_json(e: &kalango_craft::instalar::Erro) -> Value {
    use kalango_craft::instalar::Erro;
    use kalango_craft::verificar::Falha;
    match e {
        Erro::Verificacao(Falha::Bloqueada { versao, motivo }) => json!({"tipo": "bloqueada", "versao": versao, "motivo": motivo}),
        Erro::Verificacao(Falha::Assinatura(x) | Falha::Formato(x)) => json!({"tipo": "assinatura", "texto": x}),
        Erro::Verificacao(Falha::Rede(x)) => json!({"tipo": "rede", "texto": x}),
        Erro::Verificacao(Falha::Cancelada) => json!({"tipo": "cancelada"}),
        Erro::SemEspaco { faltam } => json!({"tipo": "espaco", "faltam": faltam}),
        Erro::EmUso => json!({"tipo": "em_uso"}),
        Erro::Flatpak(x) => json!({"tipo": "flatpak", "texto": x}),
    }
}

fn estado_do_app(cfg: &crate::policy::Config, uid: u32, id: &str) -> Value {
    ler_ajustes(&cfg.state_dir, uid)[id].clone()
}

/// Se a pessoa deu rede ao app (desligada por padrão).
fn rede(cfg: &crate::policy::Config, uid: u32, id: &str) -> bool {
    estado_do_app(cfg, uid, id)["rede"].as_bool().unwrap_or(false)
}

/// Os nomes das pastas escolhidas (ou as do catálogo).
fn nomes_das_pastas(cfg: &crate::policy::Config, uid: u32, app: &App) -> Vec<String> {
    estado_do_app(cfg, uid, &app.id)["pastas"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_else(|| app.pastas.clone())
}

/// Refaz as permissões do app no Flatpak da pessoa a partir do estado do kalangod: apaga o que houver (um override
/// mexido por outro programa não alarga nada) e põe rede, serviço do Flatpak e só as pastas conferidas.
pub fn aplicar_permissoes(cfg: &crate::policy::Config, peer: &Peer, app: &App) -> Result<(), String> {
    let ps = pastas(cfg, peer, app)?;
    let rodar = |args: &[String]| -> Result<String, String> {
        let o = flatpak_como(peer).args(args).env("LC_ALL", "C").stdin(std::process::Stdio::null()).output().map_err(|e| e.to_string())?;
        if o.status.success() {
            Ok(String::from_utf8_lossy(&o.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
        }
    };
    let meta = rodar(&["--user".into(), "info".into(), "--show-metadata".into(), app.id.clone()]).unwrap_or_default();
    rodar(&["override".into(), "--user".into(), "--reset".into(), app.id.clone()])?;
    let caminhos: Vec<String> = ps.iter().map(|p| p.display().to_string()).collect();
    rodar(&kalango_craft::instalar::argumentos_de_permissao(
        &app.id,
        rede(cfg, peer.uid, &app.id),
        &caminhos,
        &kalango_craft::instalar::Pedidos::de(&meta),
    ))?;
    verificar_override_global(peer, app)
}

/// O override global da pessoa não alarga o que o app vê (ver `override_global_alarga`): conferido logo depois de refazer
/// as permissões do app e de novo logo antes de iniciar o servidor. Um que alarga recusa o início, nunca é apagado (é da
/// pessoa e vale para os outros apps dela).
fn verificar_override_global(peer: &Peer, app: &App) -> Result<(), String> {
    let o = flatpak_como(peer)
        .args(["override", "--user", "--show"])
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
    }
    match kalango_craft::instalar::override_global_alarga(&String::from_utf8_lossy(&o.stdout)) {
        None => Ok(()),
        // (a chamada e o texto na mesma linha: o teste do catálogo de traduções procura o texto colado à chamada)
        #[rustfmt::skip]
        Some(item) => Err(trf("refused: a global Flatpak override of yours would widen what {app} sees ({item}); remove it with `flatpak override --user --reset`", &[("app", app.nome.clone()), ("item", item)])),
    }
}

/// Subpastas da casa que nenhum app criativo vê: chaves, configurações, dados de programas, chaveiros.
/// Nomes de pasta que nunca são dados a um app, em qualquer ponto do caminho (além de toda pasta com ponto): onde
/// ficam programas e chaves.
const NUNCA: &[&str] = &["bin", "sbin", "scripts", "keyrings", "node_modules", "snap", "Library"];

/// Uma pasta que um app pode ver — lista do que pode, não do que não pode: uma das pastas de mídia e documentos do
/// XDG (xdg-videos, xdg-pictures, xdg-documents, xdg-music, xdg-download) ou uma pasta que a pessoa escolheu
/// (`~/…`); resolvida pelo kalangod (links seguidos), uma pasta que existe (não um arquivo), dentro da casa e não a
/// própria casa, sem nenhum componente com ponto nem de programas (`NUNCA`), fora das pastas protegidas do kalangod.
/// O `user-dirs.dirs` da pessoa só vale se apontar uma pasta assim. Recusa, nunca troca por outra.
pub fn pasta_segura(cfg: &crate::policy::Config, peer: &Peer, nome: &str) -> Result<PathBuf, String> {
    let recusa = || trf("refused: the folder {f} is outside what a creative app may see", &[("f", nome.to_string())]);
    let dirs = std::fs::read_to_string(peer.home.join(".config/user-dirs.dirs")).unwrap_or_default();
    let p = caminho_da_pasta(&peer.home, &dirs, nome).ok_or_else(recusa)?;
    let casa = std::fs::canonicalize(&peer.home).map_err(|_| recusa())?;
    let real = std::fs::canonicalize(&p).map_err(|_| trf("the folder {f} does not exist", &[("f", nome.to_string())]))?;
    if !real.is_dir() || !real.starts_with(&casa) || real == casa {
        return Err(recusa());
    }
    let rel = real.strip_prefix(&casa).map_err(|_| recusa())?;
    let ruim = |c: std::path::Component| {
        let s = c.as_os_str().to_string_lossy();
        s.starts_with('.') || NUNCA.iter().any(|n| s.eq_ignore_ascii_case(n))
    };
    // (o caminho escrito e o resolvido: nenhum dos dois passa por uma pasta com ponto)
    if rel.components().any(ruim) || p.strip_prefix(&peer.home).map(|r| r.components().any(ruim)).unwrap_or(false) {
        return Err(recusa());
    }
    for prot in crate::allowlist::protected(cfg, &peer.home) {
        let prot = std::fs::canonicalize(&prot).unwrap_or(prot);
        if real.starts_with(&prot) || prot.starts_with(&real) {
            return Err(recusa());
        }
    }
    Ok(real)
}

// ------------------------------------------------------------------------------------------------ running as the person

/// O flatpak, como a pessoa (o uid e a casa dela; o barramento e a pasta de execução dela, para o Flatpak achar o
/// portal). `KALANGO_FLATPAK` (testes) troca o programa.
fn flatpak_como(peer: &Peer) -> Command {
    let mut cmd = Command::new(std::env::var("KALANGO_FLATPAK").unwrap_or_else(|_| "flatpak".into()));
    if nix::unistd::geteuid().is_root() && peer.uid != 0 {
        use std::os::unix::process::CommandExt;
        cmd.uid(peer.uid).gid(peer.gid);
    }
    // (o ambiente é só o que o kalangod põe, com ou sem root: a conferência do override e o `flatpak run` leem as mesmas
    // pastas, e nada do ambiente do kalangod — nem um FLATPAK_USER_DIR ou XDG_* — muda onde o Flatpak procura overrides)
    let casa = &peer.passwd_home;
    cmd.env_clear()
        .env("HOME", casa)
        .env("USER", &peer.user)
        .env("XDG_RUNTIME_DIR", format!("/run/user/{}", peer.uid))
        .env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path=/run/user/{}/bus", peer.uid))
        .env("XDG_DATA_HOME", casa.join(".local/share"))
        .env("XDG_CONFIG_HOME", casa.join(".config"))
        .env("XDG_CACHE_HOME", casa.join(".cache"))
        .env("XDG_STATE_HOME", casa.join(".local/state"))
        .env("FLATPAK_USER_DIR", casa.join(".local/share/flatpak"))
        .env("FLATPAK_CONFIG_DIR", "/etc/flatpak")
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("LANG", std::env::var("LANG").unwrap_or_else(|_| "C.UTF-8".into()));
    if casa.is_dir() {
        cmd.current_dir(casa);
    }
    // (uma instalação de teste do Flatpak, só numa sessão de teste: com a casa do teste, para os dados do app em
    // ~/.var/app não irem para a pasta pessoal de verdade)
    if let Some(d) = std::env::var_os("KALANGO_CRAFT_FLATPAK_USER_DIR").filter(|_| peer.home != peer.passwd_home) {
        cmd.env("FLATPAK_USER_DIR", d).env("HOME", &peer.home);
    }
    cmd
}

fn instalados(peer: &Peer) -> Vec<String> {
    let o = flatpak_como(peer).args(["--user", "list", "--app", "--columns=application"]).stderr(std::process::Stdio::null()).output();
    o.map(|o| String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).filter(|l| l.starts_with("ai.storyteller.")).collect())
        .unwrap_or_default()
}

/// As pastas de um app: as que a pessoa escolheu na Central (no estado do kalangod), senão as do catálogo; cada uma
/// conferida agora (`pasta_segura`). Uma que não passa recusa a chamada (e entra na linha do tempo como erro).
fn pastas(cfg: &crate::policy::Config, peer: &Peer, app: &App) -> Result<Vec<PathBuf>, String> {
    let v = json!({&app.id: estado_do_app(cfg, peer.uid, &app.id)});
    let nomes = nomes_das_pastas(cfg, peer.uid, app);
    let mut out = Vec::new();
    for n in &nomes {
        match pasta_segura(cfg, peer, n) {
            Ok(p) => out.push(p),
            // (uma pasta do catálogo que não existe nesta casa fica de fora; o resto recusa)
            Err(_) if v[&app.id]["pastas"].is_null() && caminho_da_pasta(&peer.home, "", n).is_some_and(|p| !p.exists()) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

/// "xdg-videos" → o XDG_VIDEOS_DIR da pessoa (ou ~/Videos); "~/x" → casa/x; um absoluto dentro da casa fica.
pub fn caminho_da_pasta(casa: &Path, user_dirs: &str, nome: &str) -> Option<PathBuf> {
    let nome = nome.split(':').next().unwrap_or(nome);
    if let Some(x) = nome.strip_prefix("xdg-") {
        let chave = format!(
            "XDG_{}_DIR=",
            match x {
                "download" => "DOWNLOAD".to_string(),
                o => o.to_ascii_uppercase(),
            }
        );
        let padrao = match x {
            "documents" => "Documents",
            "pictures" => "Pictures",
            "videos" => "Videos",
            "music" => "Music",
            "download" => "Downloads",
            _ => return None,
        };
        let v = user_dirs
            .lines()
            .find_map(|l| l.trim().strip_prefix(&chave))
            .map(|v| v.trim().trim_matches('"').replace("$HOME", &casa.to_string_lossy()));
        return Some(v.map(PathBuf::from).unwrap_or_else(|| casa.join(padrao)));
    }
    let p = if let Some(r) = nome.strip_prefix("~/") { casa.join(r) } else { PathBuf::from(nome) };
    // (só dentro da casa da pessoa, sem "..")
    (p.is_absolute() && p.starts_with(casa) && !p.components().any(|c| matches!(c, std::path::Component::ParentDir))).then_some(p)
}

/// Os argumentos do `flatpak run` que iniciam o servidor MCP de um app.
pub fn argumentos_do_servidor(app: &App, pastas: &[PathBuf], janela: Option<u16>) -> Vec<String> {
    let mut a: Vec<String> = vec!["run".into(), "--unshare=network".into()];
    // (o que o override já nega, dito de novo na própria chamada: um override mexido por fora não alarga nada)
    a.extend(kalango_craft::instalar::negacoes_fixas());
    for amplo in ["host", "home", "/media", "/run/media", "/mnt"] {
        a.push(format!("--nofilesystem={amplo}"));
    }
    for p in pastas {
        a.push(format!("--filesystem={}", p.display()));
    }
    let mut raizes: Vec<String> = Vec::new();
    if app.raizes {
        for p in pastas {
            raizes.push("--automation-read-root".into());
            raizes.push(p.display().to_string());
        }
        if let Some(w) = pastas.first() {
            raizes.push("--automation-write-root".into());
            raizes.push(w.display().to_string());
        }
    }
    match janela {
        None => {
            // sem janela: sem tela nem som
            for s in ["x11", "wayland", "fallback-x11", "pulseaudio"] {
                a.push(format!("--nosocket={s}"));
            }
            a.push(format!("--command={}", app.cli));
            a.push(app.id.clone());
            a.push("mcp".into());
            a.extend(raizes);
        }
        Some(porta) => {
            // a janela e o servidor no mesmo sandbox (a rede dele é só a dele: o loopback liga os dois)
            let gui = app.cli.trim_end_matches("-cli");
            let flag = app.janela.clone().unwrap_or_default();
            let q = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
            let mut script = format!("{} --control {porta} & exec {} mcp {} {porta}", q(gui), q(&app.cli), q(&flag));
            for r in &raizes {
                script.push(' ');
                script.push_str(&q(r));
            }
            a.push("--command=sh".into());
            a.push(app.id.clone());
            a.push("-c".into());
            a.push(script);
        }
    }
    a
}

// ------------------------------------------------------------------------------------------------ the sessions

type Chave = (u32, String, bool);

/// Uma sessão guardada: o cadeado da sessão (segurado durante uma chamada, que pode durar minutos) e o `Controle`, que
/// encerra o servidor de fora sem esperar esse cadeado.
struct Entrada {
    sessao: Mutex<kalango_craft::mcp::Sessao>,
    controle: kalango_craft::mcp::Controle,
}

#[derive(Default)]
struct Tabela {
    mapa: HashMap<Chave, std::sync::Arc<Entrada>>,
    /// quantas vezes cada (pessoa, app) foi fechado: uma sessão que começou antes de um fechamento não entra depois dele
    geracao: HashMap<(u32, String), u64>,
    /// quem está iniciando uma sessão (uma só por chave)
    partida: HashMap<Chave, std::sync::Arc<Mutex<()>>>,
}

/// (o cadeado da tabela só guarda e tira entradas: nunca é segurado durante o início de um servidor nem uma chamada)
static SESSOES: Mutex<Option<Tabela>> = Mutex::new(None);
const PARADA: Duration = Duration::from_secs(600);
const PRAZO: Duration = Duration::from_secs(120);

fn tabela() -> std::sync::MutexGuard<'static, Option<Tabela>> {
    let mut g = SESSOES.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(Tabela::default);
    g
}

/// Encerra já as sessões de um app desta pessoa (o interruptor "Agentes podem usar este app", ou a troca das pastas):
/// tira da tabela, marca o fechamento e mata o servidor, mesmo com uma chamada em curso.
fn fechar_do_app(uid: u32, id: &str) {
    let tiradas: Vec<std::sync::Arc<Entrada>> = {
        let mut g = tabela();
        let t = g.as_mut().unwrap();
        *t.geracao.entry((uid, id.to_string())).or_default() += 1;
        let chaves: Vec<Chave> = t.mapa.keys().filter(|k| k.0 == uid && k.1 == id).cloned().collect();
        chaves.iter().filter_map(|k| t.mapa.remove(k)).collect()
    };
    for e in &tiradas {
        e.controle.encerrar();
    }
}

fn ficha() -> String {
    let mut b = [0u8; 32];
    let _ = std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b));
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A sessão deste app para esta pessoa (iniciada agora se não houver ou se morreu); roda `f` com ela. O cadeado da
/// tabela nunca fica preso a um início ou a uma chamada.
fn com_sessao<T>(
    cfg: &crate::policy::Config,
    peer: &Peer,
    app: &App,
    janela: bool,
    f: impl FnOnce(&mut kalango_craft::mcp::Sessao) -> Result<T, String>,
) -> Result<T, String> {
    let k = (peer.uid, app.id.clone(), janela);
    let mut velhas = Vec::new();
    let (existente, geracao, partida) = {
        let mut g = tabela();
        let t = g.as_mut().unwrap();
        // (uma sessão em uso numa chamada fica; uma morta ou parada é tirada, e solta fora do cadeado)
        let ruins: Vec<Chave> = t
            .mapa
            .iter()
            .filter(|(_, e)| match e.sessao.try_lock() {
                Ok(mut s) => !(s.viva() && s.usada.elapsed() < PARADA),
                Err(std::sync::TryLockError::WouldBlock) => false,
                Err(std::sync::TryLockError::Poisoned(_)) => true,
            })
            .map(|(k, _)| k.clone())
            .collect();
        velhas.extend(ruins.iter().filter_map(|k| t.mapa.remove(k)));
        (t.mapa.get(&k).cloned(), t.geracao.get(&(peer.uid, app.id.clone())).copied().unwrap_or(0), t.partida.entry(k.clone()).or_default().clone())
    };
    drop(velhas);
    // (a geração foi lida ANTES desta conferência: se a pessoa desligar o interruptor depois dela, o fechamento muda a
    // geração e as conferências de baixo pegam)
    let desligado = || trf("refused: you turned off agents' use of {app} (App Center)", &[("app", app.nome.clone())]);
    if !agentes_podem(&cfg.state_dir, peer.uid, &app.id) {
        return Err(desligado());
    }
    let entrada = match existente {
        Some(e) => e,
        None => {
            // (uma partida por vez para esta chave; quem esperou usa a sessão que a outra criou)
            let _p = partida.lock().unwrap_or_else(|e| e.into_inner());
            let ja = tabela().as_ref().unwrap().mapa.get(&k).cloned();
            match ja {
                Some(e) => e,
                None => {
                    let s = iniciar_sessao(cfg, peer, app, janela)?;
                    let nova = std::sync::Arc::new(Entrada { controle: s.controle(), sessao: Mutex::new(s) });
                    let mut g = tabela();
                    let t = g.as_mut().unwrap();
                    // (sob o cadeado da tabela: o app foi fechado, ou os agentes desligados, enquanto iniciava? esta sessão não entra)
                    if t.geracao.get(&(peer.uid, app.id.clone())).copied().unwrap_or(0) != geracao
                        || !agentes_podem(&cfg.state_dir, peer.uid, &app.id)
                    {
                        drop(g);
                        nova.controle.encerrar();
                        return Err(trf("{app}: the session was closed while it started", &[("app", app.nome.clone())]));
                    }
                    t.mapa.insert(k.clone(), nova.clone());
                    nova
                }
            }
        }
    };
    let mut s = entrada.sessao.lock().unwrap_or_else(|e| e.into_inner());
    // (de novo, logo antes da chamada, com a sessão em mãos: a espera pelo cadeado dela pode ter sido longa)
    {
        let mut g = tabela();
        let t = g.as_mut().unwrap();
        let mesma = t.geracao.get(&(peer.uid, app.id.clone())).copied().unwrap_or(0) == geracao;
        if entrada.controle.encerrada() || !mesma || !agentes_podem(&cfg.state_dir, peer.uid, &app.id) {
            drop(g);
            entrada.controle.encerrar();
            return Err(trf("{app}: the session was closed", &[("app", app.nome.clone())]));
        }
    }
    f(&mut s)
}

/// Inicia o servidor do app como a pessoa (sem segurar nenhum cadeado da tabela).
fn iniciar_sessao(cfg: &crate::policy::Config, peer: &Peer, app: &App, janela: bool) -> Result<kalango_craft::mcp::Sessao, String> {
    if janela && app.janela.is_none() {
        return Err(trf("{app} cannot be driven on a window by agents", &[("app", app.nome.clone())]));
    }
    let ps = pastas(cfg, peer, app)?;
    // (as permissões do app de novo, do estado do kalangod, antes de iniciar: um override alargado por fora some)
    aplicar_permissoes(cfg, peer, app)
        .map_err(|e| trf("{app}: the permissions could not be set, so it did not start: {e}", &[("app", app.nome.clone()), ("e", e)]))?;
    if app.raizes && ps.is_empty() {
        return Err(trf("{app} sees no folder of yours: choose one in the App Center", &[("app", app.nome.clone())]));
    }
    let porta = janela.then(|| 20000 + (u16::from_str_radix(&ficha()[..4], 16).unwrap_or(0) % 40000));
    let mut cmd = flatpak_como(peer);
    cmd.args(argumentos_do_servidor(app, &ps, porta));
    if janela {
        // (a ficha só no ambiente dos dois processos que o kalangod inicia; a tela da pessoa para a janela)
        if let Some(t) = &app.token {
            cmd.env(t, ficha());
        }
        cmd.env("WAYLAND_DISPLAY", std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-1".into()));
    }
    // (logo antes de iniciar: o que foi escrito depois das permissões refeitas também vale)
    verificar_override_global(peer, app)?;
    kalango_craft::mcp::Sessao::iniciar(cmd, PRAZO).map_err(|e| trf("{app} did not start: {e}", &[("app", app.nome.clone()), ("e", e)]))
}

// ------------------------------------------------------------------------------------------------ the tools

fn app_de(args: &Value) -> Result<&'static App, String> {
    let a = s(args, "app")?;
    catalogo().app(a).ok_or_else(|| trf("{app} is not a creative app of the catalog", &[("app", a.to_string())]))
}

fn apps(c: &Ctx, args: &Value) -> Result<Out, String> {
    let inst = instalados(c.peer);
    let so = args["app"].as_str().and_then(|a| catalogo().app(a)).map(|a| a.id.clone());
    let mut lista = Vec::new();
    for a in catalogo().apps.iter().filter(|a| inst.contains(&a.id)).filter(|a| so.as_ref().is_none_or(|x| *x == a.id)) {
        let pode = agentes_podem(&c.cfg.state_dir, c.peer.uid, &a.id);
        let mut v = json!({"id": a.id, "nome": a.nome, "descricao": a.descricao, "agentes_podem_usar": pode, "janela": a.janela.is_some()});
        if args["ferramentas"] == true && pode {
            // (a versão instalada uma vez por app, não por ferramenta)
            let instalada = versao_verificada(c.cfg, c.peer, a);
            let r = com_sessao(c.cfg, c.peer, a, false, |s| Ok(s.ferramentas.clone()));
            match r {
                Ok(ts) => {
                    v["ferramentas"] = json!(ts.iter().map(|t| {
                        let n = t["name"].as_str().unwrap_or("");
                        let (nivel, motivo) = classificar(catalogo(), a, n, &json!({}), false, instalada.as_deref());
                        json!({"nome": n, "descricao": t["description"], "nivel": nivel.nome(), "porque": tr(motivo.texto()), "inputSchema": t["inputSchema"]})
                    }).collect::<Vec<_>>());
                }
                Err(e) => v["erro"] = json!(e),
            }
        }
        lista.push(v);
    }
    let n = lista.len();
    Ok(Out::read(
        json!({"apps": lista, "nao_instalados": catalogo().apps.iter().filter(|a| !inst.contains(&a.id)).map(|a| &a.nome).collect::<Vec<_>>()}),
        trf("{n} creative apps installed", &[("n", n.to_string())]),
    ))
}

/// O commit que está de fato instalado na instalação da pessoa (`flatpak info --show-commit`, como ela), se houver.
fn commit_instalado_de_verdade(peer: &Peer, id: &str) -> Option<String> {
    let o = flatpak_como(peer)
        .args(["--user", "info", "--show-commit", id])
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|c| c.len() == 64)
}

/// A versão do app que a lista de comandos livres pode usar: a que o kalangod verificou (`atual`), e só se o commit
/// realmente instalado na instalação da pessoa é o commit dessa versão. Um registro velho (o app foi atualizado ou
/// trocado por fora) ou um app sem registro dá `None`: a lista não vale e os ids pedem aprovação.
fn versao_verificada(cfg: &crate::policy::Config, peer: &Peer, app: &App) -> Option<String> {
    let (commit, versao) = verificados(cfg, peer.uid).get(&app.id).and_then(|e| e.atual.clone())?;
    (commit_instalado_de_verdade(peer, &app.id).as_deref() == Some(commit.as_str())).then_some(versao)
}

/// O nível desta chamada, antes de tudo (o servidor pergunta à pessoa com ele).
pub fn nivel_da_chamada(cfg: &crate::policy::Config, peer: &Peer, args: &Value) -> Result<(&'static App, Nivel, &'static str), String> {
    let app = app_de(args)?;
    let f = s(args, "ferramenta")?;
    let (n, m) = classificar(
        catalogo(),
        app,
        f,
        &args.get("argumentos").cloned().unwrap_or(json!({})),
        args["janela"] == true,
        versao_verificada(cfg, peer, app).as_deref(),
    );
    Ok((app, n, m.texto()))
}

/// Um texto que veio do agente, para a tela: numa linha (quebras e controles viram espaço), espaços juntados e no
/// máximo `max` caracteres. (A tela o põe num rótulo de texto simples, nunca como marcação.)
/// Caracteres que não se veem, que mudam a direção do texto ou que são "espaços" vazios: toda a categoria Unicode `Cf`
/// (formato) de hoje, escrita em faixas (a std não tem a tabela), as etiquetas U+E0000–E007F inteiras, U+2028/2029 e os
/// preenchimentos vazios (Hangul U+115F, U+1160, U+3164, U+FFA0; khmer U+17B4, U+17B5; o junta-grafemas U+034F).
fn invisivel(c: char) -> bool {
    const FAIXAS: &[(u32, u32)] = &[
        (0x00AD, 0x00AD),
        (0x034F, 0x034F),
        (0x0600, 0x0605),
        (0x061C, 0x061C),
        (0x06DD, 0x06DD),
        (0x070F, 0x070F),
        (0x0890, 0x0891),
        (0x08E2, 0x08E2),
        (0x115F, 0x1160),
        (0x17B4, 0x17B5),
        (0x180E, 0x180E),
        (0x200B, 0x200F),
        (0x2028, 0x202E),
        (0x2060, 0x206F),
        (0x3164, 0x3164),
        (0xFEFF, 0xFEFF),
        (0xFFA0, 0xFFA0),
        (0xFFF9, 0xFFFB),
        (0x110BD, 0x110BD),
        (0x110CD, 0x110CD),
        (0x13430, 0x1343F),
        (0x1BCA0, 0x1BCA3),
        (0x1D173, 0x1D17A),
        (0xE0000, 0xE007F),
    ];
    let c = c as u32;
    FAIXAS.iter().any(|&(a, b)| (a..=b).contains(&c))
}

pub fn texto_de_agente(s: &str, max: usize) -> String {
    let uma: String = s.chars().map(|c| if c.is_control() || invisivel(c) { ' ' } else { c }).collect();
    let junto = uma.split_whitespace().collect::<Vec<_>>().join(" ");
    if junto.chars().count() > max {
        format!("{}…", junto.chars().take(max).collect::<String>())
    } else {
        junto
    }
}

/// Se um caminho pedido cai dentro de uma das pastas permitidas (já canônicas): sem `..`, com a parte que existe
/// resolvida (links seguidos) e o resto acrescentado; só então comparado.
pub fn dentro_das_permitidas(casa: &Path, permitidas: &[PathBuf], c: &str) -> bool {
    let p = c.strip_prefix("~/").map(|r| casa.join(r)).unwrap_or_else(|| PathBuf::from(c));
    // (no texto pedido: Path::components esconde um "." no meio)
    if !p.is_absolute() || c.split('/').any(|s| s == "." || s == "..") {
        return false;
    }
    // o ancestral mais fundo que existe, resolvido; o que falta vem depois
    let mut base = p.clone();
    let mut resto: Vec<std::ffi::OsString> = Vec::new();
    let real = loop {
        if let Ok(r) = std::fs::canonicalize(&base) {
            break r;
        }
        match (base.file_name().map(|n| n.to_os_string()), base.parent().map(Path::to_path_buf)) {
            (Some(n), Some(pai)) => {
                resto.push(n);
                base = pai;
            }
            _ => return false,
        }
    };
    let mut final_ = real;
    for n in resto.iter().rev() {
        final_.push(n);
    }
    permitidas.iter().any(|d| final_.starts_with(d))
}

/// O que a tela de aprovação mostra de uma chamada (design 346477): o verbo e a classe do nível, O quê (a ferramenta
/// e o comando), Para onde (os caminhos, dizendo quando a pasta já é uma das permitidas), Formato (o pedido ou a
/// extensão) e Por quê (o que o agente disse em `por_que`: texto dele, mostrado como dado), e a nota sobre a janela.
/// Quantos caminhos o cartão de aprovação mostra.
const MAX_CAMINHOS: usize = 8;

pub fn cartao(cfg: &crate::policy::Config, peer: &Peer, args: &Value) -> Value {
    use kalango_craft::niveis::Motivo;
    let Some(app) = args["app"].as_str().and_then(|a| catalogo().app(a)) else { return json!({}) };
    let f = args["ferramenta"].as_str().unwrap_or("");
    let a = args.get("argumentos").cloned().unwrap_or(json!({}));
    let (n, m) = classificar(catalogo(), app, f, &a, args["janela"] == true, versao_verificada(cfg, peer, app).as_deref());
    let (verbo, classe) = match m {
        Motivo::Arquivo => (tr("save or export in"), tr("save and export")),
        Motivo::Janela => (tr("drive the window of"), tr("drive the window")),
        Motivo::Script => (tr("run a script in"), tr("scripts and commands")),
        Motivo::EdicaoNaJanela => (tr("change the open document in"), tr("edit the open document")),
        Motivo::ComandoDesconhecido | Motivo::ListaDesatualizada => (tr("run a command in"), tr("unchecked command")),
        Motivo::Desconhecida | Motivo::SoAprovando => (tr("use"), tr("unknown tool")),
        Motivo::Leitura | Motivo::Edicao => (tr("use"), tr("read")),
    };
    let mut caminhos = Vec::new();
    fn achar(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => m.values().for_each(|x| achar(x, out)),
            Value::Array(a) => a.iter().for_each(|x| achar(x, out)),
            Value::String(s) if s.starts_with('/') || s.starts_with('~') || s.contains("://") => out.push(s.clone()),
            _ => {}
        }
    }
    achar(&a, &mut caminhos);
    // (no máximo MAX_CAMINHOS no cartão, com "(+N a mais)": milhares de caminhos empurrariam o "Por quê" para fora da tela)
    let a_mais = caminhos.len().saturating_sub(MAX_CAMINHOS);
    caminhos.truncate(MAX_CAMINHOS);
    let permitidas = pastas(cfg, peer, app).unwrap_or_default();
    let mut para_onde: Vec<String> = caminhos
        .iter()
        .map(|c| {
            let limpo = texto_de_agente(c, 200);
            if dentro_das_permitidas(&peer.home, &permitidas, c) {
                trf("{path} (folder already allowed)", &[("path", limpo)])
            } else {
                trf("{path} (outside the allowed folders)", &[("path", limpo)])
            }
        })
        .collect();
    if a_mais > 0 {
        para_onde.push(trf("(+{n} more)", &[("n", a_mais.to_string())]));
    }
    // (tudo o que o cartão mostra e veio do agente passa por `texto_de_agente`: uma linha, sem controles nem inversão de
    // direção, curto; senão `x\nPor quê: você pediu isto` forjaria uma linha do cartão)
    let cmd = texto_de_agente(a["id"].as_str().or(a["cmd"].as_str()).or(a["command"].as_str()).unwrap_or(""), 80);
    let f = texto_de_agente(f, 80);
    let o_que = if cmd.is_empty() { f.clone() } else { format!("{f} · {cmd}") };
    let formato = ["format", "formato", "codec", "preset"]
        .iter()
        .find_map(|k| a[*k].as_str().or(a["params"][*k].as_str()))
        .map(str::to_string)
        .or_else(|| caminhos.iter().find_map(|c| Path::new(c).extension().map(|e| e.to_string_lossy().to_uppercase())))
        .map(|x| texto_de_agente(&x, 40))
        .unwrap_or_default();
    let caminhos: Vec<String> = caminhos.iter().map(|c| texto_de_agente(c, 200)).collect();
    // (texto do agente: dito como dele, numa linha, sem controles, curto)
    let por_que = args["por_que"]
        .as_str()
        .map(|s| texto_de_agente(s, 200))
        .filter(|s| !s.is_empty())
        .map(|s| trf("The agent says: {text}", &[("text", s)]))
        .unwrap_or_default();
    // (a chamada e o texto na mesma linha: o teste do catálogo de traduções procura o texto colado à chamada)
    #[rustfmt::skip]
    let nota = if n == Nivel::Aprovar { trf("Driving {app} or running what KalangoOS does not know: the d1 shows its confidence; never \"always\".", &[("app", app.nome.clone())]) }
        else { trf("Controlling the {app} window (clicking, dragging) would ask a separate approval, with the d1 showing its confidence.", &[("app", app.nome.clone())]) };
    json!({"app": app.nome, "ferramenta": f, "comando": cmd, "nivel": n.nome(), "porque": tr(m.texto()), "verbo": verbo, "classe": classe,
        "o_que": o_que, "para_onde": para_onde.join(" · "), "formato": formato, "por_que": por_que, "nota": nota, "caminhos": caminhos, "janela": args["janela"] == true})
}

fn chamar(c: &Ctx, args: &Value) -> Result<Out, String> {
    let app = app_de(args)?;
    if !agentes_podem(&c.cfg.state_dir, c.peer.uid, &app.id) {
        return Err(trf("refused: you turned off agents' use of {app} (App Center)", &[("app", app.nome.clone())]));
    }
    if !instalados(c.peer).contains(&app.id) {
        return Err(trf("{app} is not installed: install it in the App Center", &[("app", app.nome.clone())]));
    }
    let f = s(args, "ferramenta")?.to_string();
    let janela = args["janela"] == true;
    let a = args.get("argumentos").cloned().unwrap_or(json!({}));
    let (nivel, _) = classificar(catalogo(), app, &f, &a, janela, versao_verificada(c.cfg, c.peer, app).as_deref());
    let r = com_sessao(c.cfg, c.peer, app, janela, |s| {
        // (uma ferramenta que o servidor não anuncia não é chamada)
        // (a versão instalada de novo, com a sessão em mãos: se mudou desde a classificação, a chamada não segue)
        let (de_novo, _) = classificar(catalogo(), app, &f, &a, janela, versao_verificada(c.cfg, c.peer, app).as_deref());
        if de_novo > nivel {
            return Err(trf("{app} changed while the call was being prepared: ask again", &[("app", app.nome.clone())]));
        }
        if !s.ferramentas.iter().any(|t| t["name"] == f.as_str()) {
            return Err(trf("{app} has no tool {tool}", &[("app", app.nome.clone()), ("tool", f.clone())]));
        }
        s.chamar(&f, a.clone(), PRAZO)
    })?;
    let erro = r["isError"] == true;
    let texto: Vec<String> = r["content"].as_array().into_iter().flatten().filter_map(|x| x["text"].as_str().map(str::to_string)).collect();
    let imagem = r["content"].as_array().into_iter().flatten().find(|x| x["type"] == "image").and_then(|x| x["data"].as_str()).map(str::to_string);
    if erro {
        return Err(trf("{app} answered with an error: {e}", &[("app", app.nome.clone()), ("e", texto.join("\n"))]));
    }
    let mut v = json!({"app": app.nome, "ferramenta": f, "nivel": nivel.nome(), "texto": texto, "dados": r.get("structuredContent").cloned().unwrap_or(Value::Null)});
    if let Some(i) = imagem {
        v["imagem"] = json!(i);
    }
    Ok(Out {
        value: v,
        summary: trf("{app}: {tool} ({level})", &[("app", app.nome.clone()), ("tool", f.clone()), ("level", nivel.nome().to_string())]),
        inverse: None,
    })
}

fn encerrar(c: &Ctx, args: &Value) -> Result<Out, String> {
    let app = app_de(args)?;
    let janela = args["janela"] == true;
    let tirada = tabela().as_mut().unwrap().mapa.remove(&(c.peer.uid, app.id.clone(), janela));
    let had = tirada.is_some();
    if let Some(e) = tirada {
        e.controle.encerrar();
    }
    Ok(Out::read(json!({"encerrada": had}), trf("session with {app} ended", &[("app", app.nome.clone())])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_headless_server_runs_without_network_screen_host_or_broad_folders() {
        let app = catalogo().app("anicraft").unwrap();
        let a = argumentos_do_servidor(app, &[PathBuf::from("/home/p/Videos")], None);
        for x in [
            "--unshare=network",
            "--no-talk-name=org.freedesktop.Flatpak",
            "--nosocket=session-bus",
            "--nosocket=system-bus",
            "--nosocket=ssh-auth",
            "--nosocket=gpg-agent",
            "--nosocket=pcsc",
            "--nosocket=cups",
            "--nodevice=all",
            "--disallow=devel",
            "--nofilesystem=home",
            "--nofilesystem=host",
            "--nosocket=wayland",
            "--nosocket=x11",
            "--command=anicraft-cli",
            "--filesystem=/home/p/Videos",
        ] {
            assert!(a.contains(&x.to_string()), "{x} in {a:?}");
        }
        let i = a.iter().position(|x| x == "ai.storyteller.anicraft").unwrap();
        assert_eq!(&a[i + 1..], ["mcp", "--automation-read-root", "/home/p/Videos", "--automation-write-root", "/home/p/Videos"]);
    }

    #[test]
    fn the_window_mode_starts_app_and_server_in_one_sandbox() {
        let app = catalogo().app("filmcraft").unwrap();
        let a = argumentos_do_servidor(app, &[PathBuf::from("/home/p/Vídeos")], Some(23456));
        assert!(a.contains(&"--command=sh".to_string()));
        assert!(!a.iter().any(|x| x.starts_with("--nosocket=wayland")));
        // the window keeps its screen but not the host's buses and keys
        assert!(a.contains(&"--nosocket=session-bus".to_string()) && a.contains(&"--nosocket=ssh-auth".to_string()));
        let script = a.last().unwrap();
        assert!(script.starts_with("'filmcraft' --control 23456 & exec 'filmcraft-cli' mcp '--bridge' 23456"), "{script}");
        // the token is never in the arguments
        assert!(!a.iter().any(|x| x.contains("TOKEN")));
    }

    #[test]
    fn folders_resolve_inside_the_home_only() {
        let casa = Path::new("/home/p");
        let dirs = "XDG_VIDEOS_DIR=\"$HOME/Vídeos\"\n";
        assert_eq!(caminho_da_pasta(casa, dirs, "xdg-videos"), Some(PathBuf::from("/home/p/Vídeos")));
        assert_eq!(caminho_da_pasta(casa, "", "xdg-pictures"), Some(PathBuf::from("/home/p/Pictures")));
        assert_eq!(caminho_da_pasta(casa, "", "~/Projetos/x"), Some(PathBuf::from("/home/p/Projetos/x")));
        assert_eq!(caminho_da_pasta(casa, "", "/etc"), None);
        assert_eq!(caminho_da_pasta(casa, "", "/home/p/../q"), None);
        assert_eq!(caminho_da_pasta(casa, "", "xdg-config"), None);
        assert_eq!(caminho_da_pasta(casa, "", "xdg-desktop"), None);
    }

    fn pessoa(casa: &Path, estado: &Path) -> (crate::policy::Config, Peer) {
        let mut cfg = crate::policy::Config::system_default();
        cfg.state_dir = estado.to_path_buf();
        cfg.protected_paths.push(format!("{}/Protegida", casa.display()));
        let mut p = Peer::from_ids(std::process::id() as i32, 1000, 1000);
        p.home = casa.to_path_buf();
        (cfg, p)
    }

    #[test]
    fn agents_may_use_an_app_until_the_person_turns_it_off() {
        let d = tempfile::tempdir().unwrap();
        let (cfg, mut p) = pessoa(d.path(), &d.path().join("estado"));
        assert!(agentes_podem(&cfg.state_dir, 1000, "ai.storyteller.filmcraft"));
        rpc(&cfg, &p, &json!({"acao": "definir", "app": "ai.storyteller.filmcraft", "agentes": false})).unwrap();
        assert!(!agentes_podem(&cfg.state_dir, 1000, "ai.storyteller.filmcraft"));
        p.uid = 1001;
        assert!(agentes_podem(&cfg.state_dir, 1001, "ai.storyteller.filmcraft"), "per person");
        assert!(rpc(&cfg, &p, &json!({"acao": "definir", "app": "not-an-app", "agentes": true})).is_err());
    }

    #[test]
    fn folders_must_stay_inside_the_home_and_out_of_sensitive_places() {
        let d = tempfile::tempdir().unwrap();
        let casa = d.path().join("casa");
        for x in ["Videos", "Projetos/a", ".ssh", ".config", "Protegida/x"] {
            std::fs::create_dir_all(casa.join(x)).unwrap();
        }
        let (cfg, p) = pessoa(&casa, &d.path().join("estado"));
        assert!(pasta_segura(&cfg, &p, "xdg-videos").is_ok());
        assert!(pasta_segura(&cfg, &p, "~/Projetos/a").is_ok());
        // the home itself, outside it, its keys and settings, a protected folder
        for bad in ["~/", "/", "/etc", "~/.ssh", "~/.config", "~/Protegida/x", "~/Protegida"] {
            assert!(pasta_segura(&cfg, &p, bad).is_err(), "{bad}");
        }
        // user-dirs.dirs is the person's file (an agent may write it): XDG_PICTURES_DIR="/" or "$HOME/.ssh" is refused
        std::fs::write(casa.join(".config/user-dirs.dirs"), "XDG_PICTURES_DIR=\"/\"\nXDG_DOCUMENTS_DIR=\"$HOME/.ssh\"\n").unwrap();
        assert!(pasta_segura(&cfg, &p, "xdg-pictures").is_err());
        assert!(pasta_segura(&cfg, &p, "xdg-documents").is_err());
        // a link that leads out is resolved before the check
        std::os::unix::fs::symlink("/etc", casa.join("Videos/fora")).unwrap();
        assert!(pasta_segura(&cfg, &p, "~/Videos/fora").is_err());
        std::os::unix::fs::symlink(casa.join(".ssh"), casa.join("chaves")).unwrap();
        assert!(pasta_segura(&cfg, &p, "~/chaves").is_err());
        // the person's choice is kept by kalangod only when every folder passes
        assert!(rpc(&cfg, &p, &json!({"acao": "definir", "app": "filmcraft", "pastas": ["xdg-videos", "~/.ssh"]})).is_err());
        rpc(&cfg, &p, &json!({"acao": "definir", "app": "filmcraft", "pastas": ["~/Projetos/a"]})).unwrap();
        let app = catalogo().app("filmcraft").unwrap();
        assert_eq!(pastas(&cfg, &p, app).unwrap(), vec![std::fs::canonicalize(casa.join("Projetos/a")).unwrap()]);
        // only the media and documents XDG folders; never a dot folder, a program folder or a file
        std::fs::create_dir_all(casa.join("Projetos/.oculta")).unwrap();
        std::fs::create_dir_all(casa.join("Projetos/bin")).unwrap();
        std::fs::write(casa.join("Projetos/nota.txt"), "x").unwrap();
        for bad in ["xdg-desktop", "xdg-config", "~/Projetos/.oculta", "~/Projetos/bin", "~/Projetos/nota.txt"] {
            assert!(pasta_segura(&cfg, &p, bad).is_err(), "{bad}");
        }
        // user-dirs pointing at a program folder under the home is refused too
        std::fs::create_dir_all(casa.join(".local/bin")).unwrap();
        std::fs::write(casa.join(".config/user-dirs.dirs"), "XDG_MUSIC_DIR=\"$HOME/.local/bin\"\n").unwrap();
        assert!(pasta_segura(&cfg, &p, "xdg-music").is_err());
        // a choice that went bad later (the folder became a link to the keys) refuses the call, never falls back
        std::fs::remove_dir_all(casa.join("Projetos/a")).unwrap();
        std::os::unix::fs::symlink(casa.join(".ssh"), casa.join("Projetos/a")).unwrap();
        assert!(pastas(&cfg, &p, app).is_err());
    }

    #[test]
    fn a_home_settings_file_has_no_effect_on_the_folders() {
        let d = tempfile::tempdir().unwrap();
        let casa = d.path().join("casa");
        for x in ["Videos", "Pictures", "Documents", ".config/kalango"] {
            std::fs::create_dir_all(casa.join(x)).unwrap();
        }
        // what an agent could write in the person's home
        std::fs::write(
            casa.join(".config/kalango/apps-criativos.json"),
            r#"{"apps": {"ai.storyteller.filmcraft": {"pastas": ["/", "~/.ssh"], "rede": true}}}"#,
        )
        .unwrap();
        let (cfg, p) = pessoa(&casa, &d.path().join("estado"));
        let app = catalogo().app("filmcraft").unwrap();
        assert_eq!(pastas(&cfg, &p, app).unwrap(), vec![std::fs::canonicalize(casa.join("Videos")).unwrap()]);
        assert!(!rede(&cfg, p.uid, &app.id));
    }

    #[test]
    fn permissions_are_reset_before_they_are_set() {
        let _g = AMBIENTE.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        let casa = d.path().join("casa");
        std::fs::create_dir_all(casa.join("Videos")).unwrap();
        let log = d.path().join("flatpak.log");
        let fake = d.path().join("flatpak");
        std::fs::write(&fake, format!("#!/bin/sh\necho \"$*\" >> {}\n", log.display())).unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let (cfg, p) = pessoa(&casa, &d.path().join("estado"));
        let app = catalogo().app("filmcraft").unwrap();
        // (the stand-in flatpak for this test only: KALANGO_FLATPAK is read when the command is made)
        std::env::set_var("KALANGO_FLATPAK", &fake);
        let r = aplicar_permissoes(&cfg, &p, app);
        std::env::remove_var("KALANGO_FLATPAK");
        r.unwrap();
        let l = std::fs::read_to_string(&log).unwrap();
        let linhas: Vec<&str> = l.lines().collect();
        let reset = linhas.iter().position(|x| x.contains("override --user --reset ai.storyteller.filmcraft")).expect("reset");
        let set = linhas.iter().position(|x| x.contains("--unshare=network") && x.contains("--nofilesystem=home")).expect("set");
        assert!(reset < set, "{l}");
        assert!(linhas[set].contains(&format!("--filesystem={}", std::fs::canonicalize(casa.join("Videos")).unwrap().display())), "{l}");
    }

    /// (tests that change KALANGO_FLATPAK run one at a time)
    static AMBIENTE: Mutex<()> = Mutex::new(());

    #[test]
    fn only_commits_kalangod_verified_become_installed_or_restore_points() {
        let _g = AMBIENTE.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        let fake = d.path().join("flatpak");
        std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        std::env::set_var("KALANGO_FLATPAK", &fake);
        let (cfg, p) = pessoa(d.path(), &d.path().join("estado"));
        let (a, b) = ("a".repeat(64), "b".repeat(64));
        let app = catalogo().app("anicraft").unwrap();
        // a commit nobody prepared (planted in the person's own state, or made up) is refused
        let r = rpc(&cfg, &p, &json!({"acao": "instalado", "app": "anicraft", "commit": a}));
        assert!(r.is_err());
        // prepared by kalangod (protected state) → installed → the next one makes it a restore point
        let repo = repositorio(&cfg);
        registrar_preparado(&cfg, p.uid, &repo, &app.id, &a, "0.2.1", "sha256:x").unwrap();
        rpc(&cfg, &p, &json!({"acao": "instalado", "app": "anicraft", "commit": a})).unwrap();
        registrar_preparado(&cfg, p.uid, &repo, &app.id, &b, "0.2.2", "sha256:y").unwrap();
        let v = rpc(&cfg, &p, &json!({"acao": "instalado", "app": "anicraft", "commit": b})).unwrap();
        assert_eq!(v["pontos"][0][0], a);
        // going back: only to the restore point kalangod keeps
        assert_eq!(rpc(&cfg, &p, &json!({"acao": "anterior", "app": "anicraft"})).unwrap()["commit"], a);
        assert!(rpc(&cfg, &p, &json!({"acao": "voltou", "app": "anicraft", "commit": "c".repeat(64)})).is_err());
        rpc(&cfg, &p, &json!({"acao": "voltou", "app": "anicraft", "commit": a})).unwrap();
        assert_eq!(verificados(&cfg, p.uid)[&app.id].atual.as_ref().unwrap().0, a);
        std::env::remove_var("KALANGO_FLATPAK");
    }

    #[test]
    fn a_destination_is_allowed_only_after_resolving_it() {
        let d = tempfile::tempdir().unwrap();
        let casa = std::fs::canonicalize(d.path()).unwrap();
        std::fs::create_dir_all(casa.join("Videos")).unwrap();
        std::fs::create_dir_all(casa.join(".ssh")).unwrap();
        std::os::unix::fs::symlink(casa.join(".ssh"), casa.join("Videos/atalho")).unwrap();
        let ok = vec![casa.join("Videos")];
        assert!(dentro_das_permitidas(&casa, &ok, "~/Videos/novo/clipe.mp4"));
        assert!(dentro_das_permitidas(&casa, &ok, &format!("{}/Videos/a.png", casa.display())));
        // `..`, a link inside the allowed folder, relative or outside paths: not "already allowed"
        for bad in ["~/Videos/../.ssh/x", "~/Videos/./a", "~/Videos/atalho/id_ed25519", "Videos/a.png", "/etc/passwd", "~/Videosx/a"] {
            assert!(!dentro_das_permitidas(&casa, &ok, bad), "{bad}");
        }
    }

    #[test]
    fn agent_text_is_one_short_plain_line() {
        assert_eq!(texto_de_agente("linha 1\nlinha 2\t\u{7}fim", 100), "linha 1 linha 2 fim");
        assert_eq!(texto_de_agente("a\u{202E}b", 10), "a b");
        assert_eq!(texto_de_agente(&"x".repeat(500), 200).chars().count(), 201);
        assert_eq!(texto_de_agente("<b>negrito</b> &amp;", 100), "<b>negrito</b> &amp;", "kept as text; the screen does not use markup");
    }

    #[test]
    fn the_call_level_comes_from_the_catalog() {
        let d = tempfile::tempdir().unwrap();
        let casa = d.path().join("casa");
        std::fs::create_dir_all(casa.join("Videos")).unwrap();
        let (cfg, p) = pessoa(&casa, &d.path().join("estado"));
        let n = |f: &str, a: Value| nivel_da_chamada(&cfg, &p, &json!({"app": "filmcraft", "ferramenta": f, "argumentos": a})).unwrap().1;
        assert_eq!(n("doc_inspect", json!({})), Nivel::Livre);
        assert_eq!(n("media_import", json!({"path": "/home/p/a.mp4"})), Nivel::Confirmar);
        assert_eq!(n("ui_click", json!({})), Nivel::Aprovar);
        assert_eq!(n("a_tool_from_tomorrow", json!({})), Nivel::Aprovar);
        let alvo = format!("{}/Videos/a.mp4", std::fs::canonicalize(&casa).unwrap().display());
        let c = cartao(
            &cfg,
            &p,
            &json!({"app": "filmcraft", "ferramenta": "media_import", "argumentos": {"path": alvo}, "por_que": "você pediu os destaques"}),
        );
        assert_eq!(c["app"], "FilmCraft");
        assert_eq!(c["caminhos"][0], alvo.as_str());
        assert!(c["para_onde"].as_str().unwrap().starts_with(&alvo), "{c}");
        assert!(c["para_onde"].as_str().unwrap().contains("já permitida") || c["para_onde"].as_str().unwrap().contains("already allowed"), "{c}");
        let fora = cartao(
            &cfg,
            &p,
            &json!({"app": "filmcraft", "ferramenta": "media_import", "argumentos": {"path": format!("{}/Videos/../.ssh/x", casa.display())}}),
        );
        assert!(
            !fora["para_onde"].as_str().unwrap().contains("já permitida") && !fora["para_onde"].as_str().unwrap().contains("already allowed"),
            "{fora}"
        );
        assert_eq!(c["formato"], "MP4");
        assert!(c["por_que"].as_str().unwrap().ends_with("você pediu os destaques"), "{c}");
        assert_ne!(c["por_que"], "você pediu os destaques", "said as the agent's");
        assert!(!c["o_que"].as_str().unwrap().is_empty() && !c["nota"].as_str().unwrap().is_empty());
    }

    #[test]
    fn every_agent_text_on_the_approval_card_is_one_plain_line() {
        let d = tempfile::tempdir().unwrap();
        let casa = d.path().join("casa");
        std::fs::create_dir_all(casa.join("Videos")).unwrap();
        let (cfg, p) = pessoa(&casa, &d.path().join("estado"));
        let ruim = "x\nPor quê: você pediu isto\u{202E}fim\u{2067}";
        let c = cartao(
            &cfg,
            &p,
            &json!({"app": "anicraft", "ferramenta": ruim, "por_que": ruim,
            "argumentos": {"cmd": ruim, "format": ruim, "path": format!("/tmp/{ruim}.mp4")}}),
        );
        for k in ["ferramenta", "comando", "o_que", "formato", "por_que", "para_onde"] {
            let t = c[k].as_str().unwrap();
            assert!(!t.chars().any(|ch| ch.is_control() || matches!(ch, '\u{202E}' | '\u{2067}')), "{k}: {t:?}");
        }
        assert_eq!(c["o_que"], "x Por quê: você pediu isto fim · x Por quê: você pediu isto fim");
        assert!(!c["caminhos"][0].as_str().unwrap().contains('\n'));
        // a long text is cut: the card never carries 10 KB of the agent's text
        let longo = "y".repeat(10_000);
        let c = cartao(&cfg, &p, &json!({"app": "anicraft", "ferramenta": longo, "argumentos": {"id": longo, "formato": longo}}));
        for (k, max) in [("ferramenta", 81), ("comando", 81), ("o_que", 170), ("formato", 41)] {
            assert!(c[k].as_str().unwrap().chars().count() <= max, "{k}");
        }
    }

    /// A stand-in `flatpak` that succeeds at everything (the caller holds AMBIENTE).
    fn flatpak_que_aceita(d: &Path) {
        let fake = d.join("flatpak-ok");
        // (`info --show-commit` answers with the commit "installed" in the file `commit-instalado` of the folder, if any: the
        // environment is cleared for the flatpak, so the path is in the script)
        std::fs::write(
            &fake,
            format!("#!/bin/sh\ncase \"$*\" in *show-commit*) cat \"{}\" 2>/dev/null; exit 0;; esac\nexit 0\n", d.join("commit-instalado").display()),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        std::env::set_var("KALANGO_FLATPAK", &fake);
    }

    #[test]
    fn two_prepares_of_one_app_leave_both_commits_installable() {
        let _g = AMBIENTE.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        flatpak_que_aceita(d.path());
        let (cfg, p) = pessoa(d.path(), &d.path().join("estado"));
        let (a, b) = ("a".repeat(64), "b".repeat(64));
        let repo = repositorio(&cfg);
        // a pending update (the app is open), then "Procurar agora" prepares again: the first is still installable
        registrar_preparado(&cfg, p.uid, &repo, "ai.storyteller.anicraft", &a, "0.3.0", "sha256:a").unwrap();
        registrar_preparado(&cfg, p.uid, &repo, "ai.storyteller.anicraft", &b, "0.3.1", "sha256:b").unwrap();
        rpc(&cfg, &p, &json!({"acao": "instalado", "app": "anicraft", "commit": a})).unwrap();
        assert_eq!(verificados(&cfg, p.uid)["ai.storyteller.anicraft"].atual.as_ref().unwrap().0, a);
        // the second one is still prepared; one that was never prepared is not
        rpc(&cfg, &p, &json!({"acao": "instalado", "app": "anicraft", "commit": b})).unwrap();
        assert!(rpc(&cfg, &p, &json!({"acao": "instalado", "app": "anicraft", "commit": "c".repeat(64)})).is_err());
        // an installed one is used up
        assert!(rpc(&cfg, &p, &json!({"acao": "instalado", "app": "anicraft", "commit": b})).is_err());
        // the repository keeps a reference for what is prepared and not yet installed
        registrar_preparado(&cfg, p.uid, &repo, "ai.storyteller.anicraft", &"d".repeat(64), "0.3.2", "sha256:d").unwrap();
        let ref0 = repo.join(format!("refs/heads/kalango-preparado/{}/ai.storyteller.anicraft/0", p.uid));
        assert_eq!(std::fs::read_to_string(ref0).unwrap().trim(), "d".repeat(64));
        std::env::remove_var("KALANGO_FLATPAK");
    }

    #[test]
    fn concurrent_installs_and_settings_never_undo_each_other() {
        let _g = AMBIENTE.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        flatpak_que_aceita(d.path());
        let (cfg, p) = pessoa(d.path(), &d.path().join("estado"));
        let ids: Vec<String> = catalogo().apps.iter().take(8).map(|a| a.id.clone()).collect();
        std::thread::scope(|sc| {
            for (n, id) in ids.iter().enumerate() {
                let (cfg, p) = (&cfg, &p);
                sc.spawn(move || {
                    let repo = repositorio(cfg);
                    let c = format!("{n:x}").repeat(64)[..64].to_string();
                    for _ in 0..5 {
                        registrar_preparado(cfg, p.uid, &repo, id, &c, "1.0.0", "sha256:x").unwrap();
                        rpc(cfg, p, &json!({"acao": "definir", "app": id, "agentes": false})).unwrap();
                        rpc(cfg, p, &json!({"acao": "instalado", "app": id, "commit": c})).unwrap();
                    }
                });
            }
        });
        let est = verificados(&cfg, p.uid);
        for (n, id) in ids.iter().enumerate() {
            assert_eq!(est[id].atual.as_ref().unwrap().0, format!("{n:x}").repeat(64)[..64], "{id}: the install of one app was undone by another");
            assert!(!agentes_podem(&cfg.state_dir, p.uid, id), "{id}: the switch was undone by another");
        }
        std::env::remove_var("KALANGO_FLATPAK");
    }

    #[test]
    fn a_prepare_is_one_per_person_and_app_with_a_folder_per_person() {
        let a = PreparoEmCurso::tentar(4001, "ai.storyteller.anicraft").expect("first");
        assert!(PreparoEmCurso::tentar(4001, "ai.storyteller.anicraft").is_none(), "the same person and app: refused while it runs");
        let outra_pessoa = PreparoEmCurso::tentar(4002, "ai.storyteller.anicraft");
        let outro_app = PreparoEmCurso::tentar(4001, "ai.storyteller.filmcraft");
        assert!(outra_pessoa.is_some() && outro_app.is_some());
        drop(a);
        assert!(PreparoEmCurso::tentar(4001, "ai.storyteller.anicraft").is_some(), "free again once it ends");
        let cfg = crate::policy::Config::system_default();
        assert_ne!(pasta_de_download(&cfg, 4001), pasta_de_download(&cfg, 4002));
        // a prepare that is already running answers with a clear state, not a clobbered download
        let (_g, d) = (AMBIENTE.lock().unwrap_or_else(|e| e.into_inner()), tempfile::tempdir().unwrap());
        let (cfg, mut p) = pessoa(d.path(), &d.path().join("estado"));
        p.uid = 4003;
        let _em_curso = PreparoEmCurso::tentar(4003, "ai.storyteller.anicraft").unwrap();
        let r = rpc(&cfg, &p, &json!({"acao": "preparar", "app": "anicraft"})).unwrap();
        assert_eq!(r["erro"]["tipo"], "em_andamento", "{r}");
    }

    #[test]
    fn the_off_switch_acts_at_once_even_with_a_call_stuck_in_the_app() {
        let _g = AMBIENTE.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        let casa = d.path().join("casa");
        std::fs::create_dir_all(casa.join("Videos")).unwrap();
        // a stand-in flatpak: `run` is an MCP server that answers the handshake and then never answers a tool call
        let fake = d.path().join("flatpak");
        std::fs::write(
            &fake,
            r#"#!/bin/sh
[ "$1" = run ] || exit 0
while IFS= read -r l; do
  id=$(printf '%s' "$l" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$l" in
    *'"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18"}}\n' "$id";;
    *'"tools/list"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"doc_inspect"}]}}\n' "$id";;
    *'"tools/call"'*) exec sleep 300;;
  esac
done
"#,
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        std::env::set_var("KALANGO_FLATPAK", &fake);
        let (cfg, mut p) = pessoa(&casa, &d.path().join("estado"));
        // (a uid of its own: the table of sessions is shared by the tests that run at once)
        p.uid = 4201;
        let app = catalogo().app("filmcraft").unwrap();
        let t = std::time::Instant::now();
        std::thread::scope(|sc| {
            let h = sc.spawn(|| com_sessao(&cfg, &p, app, false, |s| s.chamar("doc_inspect", json!({}), PRAZO)));
            // (the call is in flight: the session holds its own lock, the table is free)
            std::thread::sleep(Duration::from_millis(1500));
            let desligar = std::time::Instant::now();
            rpc(&cfg, &p, &json!({"acao": "definir", "app": "filmcraft", "agentes": false})).unwrap();
            assert!(desligar.elapsed() < Duration::from_secs(2), "the off switch waited for the call: {:?}", desligar.elapsed());
            let r = h.join().unwrap();
            assert!(r.is_err(), "the call in flight ends with an error: {r:?}");
        });
        assert!(t.elapsed() < Duration::from_secs(20), "{:?}", t.elapsed());
        assert!(tabela().as_ref().unwrap().mapa.is_empty(), "no session stays");
        std::env::remove_var("KALANGO_FLATPAK");
    }

    #[test]
    fn a_free_command_asks_again_once_auto_update_installs_another_version() {
        let _g = AMBIENTE.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        flatpak_que_aceita(d.path());
        let (cfg, p) = pessoa(d.path(), &d.path().join("estado"));
        let app = catalogo().app("anicraft").unwrap();
        let id = app.comandos_livres.ids.iter().find(|i| !i.to_lowercase().contains("save") && !i.to_lowercase().contains("file")).unwrap().clone();
        let args = json!({"app": "anicraft", "ferramenta": "anicraft_run", "argumentos": {"cmd": id}});
        let repo = repositorio(&cfg);
        let instalar = |commit: &str, versao: &str| {
            registrar_preparado(&cfg, p.uid, &repo, &app.id, commit, versao, "sha256:x").unwrap();
            rpc(&cfg, &p, &json!({"acao": "instalado", "app": "anicraft", "commit": commit})).unwrap();
        };
        let instalado = |c: &str| std::fs::write(d.path().join("commit-instalado"), c).unwrap();
        // nothing verified installed: the list is not applied
        assert_eq!(nivel_da_chamada(&cfg, &p, &args).unwrap().1, Nivel::Aprovar);
        instalar(&"a".repeat(64), &app.comandos_livres.versao);
        instalado(&"a".repeat(64));
        assert_eq!(nivel_da_chamada(&cfg, &p, &args).unwrap().1, Nivel::Livre, "the version the list was checked on, really installed");
        // the record says 0.2.1 but what is really installed is something else (updated or swapped from outside), or nothing:
        // the recorded entry is not trusted
        instalado(&"f".repeat(64));
        assert_eq!(nivel_da_chamada(&cfg, &p, &args).unwrap().1, Nivel::Aprovar, "a stale record fails closed");
        instalado("");
        assert_eq!(nivel_da_chamada(&cfg, &p, &args).unwrap().1, Nivel::Aprovar, "not installed at all");
        // 0.3 arrives by auto-update: the same id asks, and the card says why
        instalar(&"b".repeat(64), "0.3.0");
        instalado(&"b".repeat(64));
        assert_eq!(nivel_da_chamada(&cfg, &p, &args).unwrap().1, Nivel::Aprovar);
        let c = cartao(&cfg, &p, &args);
        assert_eq!(c["nivel"], "aprovar");
        assert!(c["porque"].as_str().unwrap().contains("another version") || c["porque"].as_str().unwrap().contains("outra versão"), "{c}");
        std::env::remove_var("KALANGO_FLATPAK");
    }

    #[test]
    fn a_session_is_not_used_when_agents_were_turned_off_after_the_caller_checked() {
        let _g = AMBIENTE.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        let casa = d.path().join("casa");
        std::fs::create_dir_all(casa.join("Videos")).unwrap();
        // a stand-in server that takes a second to answer the handshake
        let fake = d.path().join("flatpak");
        std::fs::write(
            &fake,
            r#"#!/bin/sh
[ "$1" = run ] || exit 0
sleep 1
while IFS= read -r l; do
  id=$(printf '%s' "$l" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$l" in
    *'"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18"}}\n' "$id";;
    *'"tools/list"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"doc_inspect"}]}}\n' "$id";;
    *'"tools/call"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[]}}\n' "$id";;
  esac
done
"#,
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        std::env::set_var("KALANGO_FLATPAK", &fake);
        let (cfg, mut p) = pessoa(&casa, &d.path().join("estado"));
        // (a uid of its own: the table of sessions is shared by the tests that run at once)
        p.uid = 4202;
        let app = catalogo().app("filmcraft").unwrap();
        let desligar = || gravar_atomico(&ajustes_path(&cfg.state_dir, p.uid), &json!({app.id.clone(): {"agentes": false}}).to_string()).unwrap();
        // off already: com_sessao itself refuses, whatever the caller checked before
        desligar();
        let mut rodou = false;
        assert!(com_sessao(&cfg, &p, app, false, |_| {
            rodou = true;
            Ok(())
        })
        .is_err());
        assert!(!rodou);
        // turned off while the server starts (the setting changes, nobody closes the app): the session does not run the call
        gravar_atomico(&ajustes_path(&cfg.state_dir, p.uid), &json!({}).to_string()).unwrap();
        let rodou = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|sc| {
            let h = sc.spawn(|| {
                com_sessao(&cfg, &p, app, false, |_| {
                    rodou.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                })
            });
            std::thread::sleep(Duration::from_millis(400));
            desligar();
            assert!(h.join().unwrap().is_err());
        });
        assert!(!rodou.load(std::sync::atomic::Ordering::SeqCst), "the call did not run");
        assert!(tabela().as_ref().unwrap().mapa.is_empty());
        std::env::remove_var("KALANGO_FLATPAK");
    }

    /// The real `flatpak` is needed by the tests that measure its effective result: without it they fail, unless
    /// KALANGO_TESTE_SEM_FLATPAK=1 says this machine has none (then they are skipped, loudly).
    fn exigir_flatpak() -> bool {
        if Command::new("flatpak").arg("--version").output().is_ok() {
            return true;
        }
        assert!(
            std::env::var_os("KALANGO_TESTE_SEM_FLATPAK").is_some(),
            "flatpak is not installed: install it, or set KALANGO_TESTE_SEM_FLATPAK=1 to skip these tests"
        );
        eprintln!("SKIPPED: no flatpak on this machine");
        false
    }

    #[test]
    fn a_global_flatpak_override_that_widens_stops_the_app_from_starting() {
        let _g = AMBIENTE.lock().unwrap_or_else(|e| e.into_inner());
        if !exigir_flatpak() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let casa = d.path().join("casa");
        for x in ["Videos", "segredos"] {
            std::fs::create_dir_all(casa.join(x)).unwrap();
        }
        let scratch = d.path().join("flatpak");
        std::env::remove_var("KALANGO_FLATPAK");
        std::env::set_var("KALANGO_CRAFT_FLATPAK_USER_DIR", &scratch);
        let real = |args: &[&str]| {
            let o = Command::new("flatpak").args(args).env("FLATPAK_USER_DIR", &scratch).env("LC_ALL", "C").output().unwrap();
            assert!(o.status.success(), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
            String::from_utf8_lossy(&o.stdout).into_owned()
        };
        let (cfg, p) = pessoa(&casa, &d.path().join("estado"));
        let app = catalogo().app("filmcraft").unwrap();
        assert_ne!(p.home, p.passwd_home, "the test installation is the one used");
        // what a program of the person could write: a global override that opens a decoy folder to every app
        let decoy = format!("--filesystem={}", casa.join("segredos").display());
        real(&["override", "--user", &decoy]);
        assert!(real(&["override", "--user", "--show"]).contains("segredos"));
        let e = aplicar_permissoes(&cfg, &p, app).unwrap_err();
        assert!(e.contains("global Flatpak override") || e.contains("override global"), "{e}");
        assert!(verificar_override_global(&p, app).is_err());
        assert!(iniciar_sessao(&cfg, &p, app, false).is_err(), "the server does not start");
        // the app's own override was still made without the decoy
        assert!(!real(&["override", "--user", "--show", &app.id]).contains("segredos"));
        // the person removes it: everything works again
        real(&["override", "--user", "--reset"]);
        aplicar_permissoes(&cfg, &p, app).unwrap();
        verificar_override_global(&p, app).unwrap();
        // a global that only denies is fine
        real(&["override", "--user", "--nofilesystem=host"]);
        verificar_override_global(&p, app).unwrap();
        std::env::remove_var("KALANGO_CRAFT_FLATPAK_USER_DIR");
    }

    #[test]
    fn the_flatpak_overrides_of_the_person_are_protected_from_agents() {
        let cfg = crate::policy::Config::system_default();
        let home = Path::new("/home/p");
        let prot = crate::allowlist::protected(&cfg, home);
        for p in ["/home/p/.local/share/flatpak/overrides", "/home/p/.config/flatpak"] {
            assert!(prot.iter().any(|x| x == Path::new(p)), "{p}");
        }
    }

    #[test]
    fn the_card_shows_a_few_paths_and_keeps_why_on_screen() {
        let d = tempfile::tempdir().unwrap();
        let casa = d.path().join("casa");
        std::fs::create_dir_all(casa.join("Videos")).unwrap();
        let (cfg, p) = pessoa(&casa, &d.path().join("estado"));
        let muitos: Vec<String> = (0..5000).map(|i| format!("/tmp/{i}/{}", "x".repeat(300))).collect();
        let c = cartao(
            &cfg,
            &p,
            &json!({"app": "filmcraft", "ferramenta": "media_import", "por_que": "você pediu os destaques", "argumentos": {"paths": muitos}}),
        );
        assert_eq!(c["caminhos"].as_array().unwrap().len(), MAX_CAMINHOS);
        assert!(c["para_onde"].as_str().unwrap().contains("4992"), "says how many more: {}", c["para_onde"]);
        assert!(c.to_string().len() < 8_000, "{} bytes", c.to_string().len());
        assert!(c["por_que"].as_str().unwrap().contains("você pediu os destaques"));
        // few paths: nothing is added
        let c = cartao(&cfg, &p, &json!({"app": "filmcraft", "ferramenta": "media_import", "argumentos": {"paths": ["/tmp/a.mp4", "/tmp/b.mp4"]}}));
        assert!(!c["para_onde"].as_str().unwrap().contains("more") && !c["para_onde"].as_str().unwrap().contains("a mais"));
    }

    #[test]
    fn invisible_and_direction_characters_never_reach_the_card() {
        // one class per line: zero-width and marks, direction controls, invisible operators, soft hyphen, Arabic and
        // Syriac format characters, Mongolian vowel separator, interlinear annotation, tags, historic and musical format
        // characters, and the blank fillers
        let faixas: &[(u32, u32)] = &[
            (0x200B, 0x200F),
            (0x202A, 0x202E),
            (0x2066, 0x2069),
            (0x2060, 0x2064),
            (0x206A, 0x206F),
            (0xFEFF, 0xFEFF),
            (0x00AD, 0x00AD),
            (0x061C, 0x061C),
            (0x0600, 0x0605),
            (0x06DD, 0x06DD),
            (0x070F, 0x070F),
            (0x08E2, 0x08E2),
            (0x180E, 0x180E),
            (0xFFF9, 0xFFFB),
            (0xE0000, 0xE007F),
            (0x110BD, 0x110BD),
            (0x1BCA0, 0x1BCA3),
            (0x1D173, 0x1D17A),
            (0x2028, 0x2029),
            (0x115F, 0x1160),
            (0x3164, 0x3164),
            (0xFFA0, 0xFFA0),
            (0x17B4, 0x17B5),
            (0x034F, 0x034F),
        ];
        for &(a, b) in faixas {
            for n in a..=b {
                let c = char::from_u32(n).unwrap();
                assert_eq!(texto_de_agente(&format!("a{c}b"), 10), "a b", "U+{n:04X}");
            }
        }
        // what is visible stays, including letters next to the ranges
        for c in ['a', 'ã', 'é', '—', '日', '😀', '\u{00AC}', '\u{00AE}', '\u{2010}', '\u{2070}'] {
            assert_eq!(texto_de_agente(&format!("x{c}y"), 10), format!("x{c}y"), "U+{:04X}", c as u32);
        }
        assert_eq!(texto_de_agente("ação — não é nada", 40), "ação — não é nada", "ordinary text is kept");
    }

    #[test]
    fn a_damaged_settings_file_turns_agents_off_instead_of_on() {
        let d = tempfile::tempdir().unwrap();
        let (cfg, p) = pessoa(d.path(), &d.path().join("estado"));
        let f = ajustes_path(&cfg.state_dir, p.uid);
        let id = "ai.storyteller.filmcraft";
        assert!(agentes_podem(&cfg.state_dir, p.uid, id), "no file: the defaults");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        for ruim in ["", "{\"ai.storyteller.filmcraft\": {\"agentes\": tr", "[]", "null", "\0\0\0"] {
            std::fs::write(&f, ruim).unwrap();
            assert!(!agentes_podem(&cfg.state_dir, p.uid, id), "{ruim:?}");
            assert!(!agentes_podem(&cfg.state_dir, p.uid, "ai.storyteller.anicraft"), "{ruim:?}");
        }
        // the person choosing again repairs the file (that app on, the others stay off until chosen)
        rpc(&cfg, &p, &json!({"acao": "definir", "app": "filmcraft", "agentes": true})).unwrap();
        assert!(agentes_podem(&cfg.state_dir, p.uid, id));
        assert!(!agentes_podem(&cfg.state_dir, p.uid, "ai.storyteller.anicraft"));
        assert!(serde_json::from_str::<Value>(&std::fs::read_to_string(&f).unwrap()).is_ok());
    }

    #[test]
    fn the_atomic_write_replaces_the_file_whole_and_leaves_nothing_behind() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("sub/estado.json");
        gravar_atomico(&f, "um").unwrap();
        gravar_atomico(&f, "dois").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "dois");
        let nomes: Vec<_> = std::fs::read_dir(d.path().join("sub")).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(nomes.len(), 1, "no temporary file stays: {nomes:?}");
        // a directory in the way of the rename: an error, and the temporary file is removed
        let em_cima = d.path().join("sub/dir");
        std::fs::create_dir_all(em_cima.join("x")).unwrap();
        assert!(gravar_atomico(&em_cima, "x").is_err());
        assert_eq!(std::fs::read_dir(d.path().join("sub")).unwrap().count(), 2);
        // many writers at once: the file is always one whole text
        std::thread::scope(|sc| {
            for n in 0..8 {
                let f = &f;
                sc.spawn(move || {
                    for _ in 0..20 {
                        gravar_atomico(f, &format!("{{\"n\": {n}}}")).unwrap();
                        assert!(serde_json::from_str::<Value>(&std::fs::read_to_string(f).unwrap()).is_ok());
                    }
                });
            }
        });
    }

    #[test]
    fn settings_changes_from_many_threads_are_all_kept() {
        let _g = AMBIENTE.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        flatpak_que_aceita(d.path());
        let (cfg, p) = pessoa(d.path(), &d.path().join("estado"));
        let ids: Vec<String> = catalogo().apps.iter().take(16).map(|a| a.id.clone()).collect();
        std::thread::scope(|sc| {
            for id in &ids {
                let (cfg, p) = (&cfg, &p);
                sc.spawn(move || {
                    for _ in 0..20 {
                        rpc(cfg, p, &json!({"acao": "definir", "app": id, "agentes": false})).unwrap();
                    }
                });
            }
        });
        for id in &ids {
            assert!(!agentes_podem(&cfg.state_dir, p.uid, id), "{id}: a change was lost");
        }
        std::env::remove_var("KALANGO_FLATPAK");
    }

    #[test]
    fn only_the_newest_prepared_commits_are_kept() {
        let _g = AMBIENTE.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        flatpak_que_aceita(d.path());
        let (cfg, p) = pessoa(d.path(), &d.path().join("estado"));
        let repo = repositorio(&cfg);
        let id = "ai.storyteller.anicraft";
        let cs: Vec<String> = ["1", "2", "3", "4"].iter().map(|c| c.repeat(64)).collect();
        for (n, c) in cs.iter().enumerate() {
            registrar_preparado(&cfg, p.uid, &repo, id, c, &format!("0.3.{n}"), "sha256:x").unwrap();
        }
        let lista: Vec<String> = verificados(&cfg, p.uid)[id].preparados.iter().map(|x| x.0.clone()).collect();
        assert_eq!(lista, vec![cs[3].clone(), cs[2].clone(), cs[1].clone()], "newest first, the oldest evicted");
        assert!(rpc(&cfg, &p, &json!({"acao": "instalado", "app": "anicraft", "commit": cs[0]})).is_err());
        // preparing the same commit again moves it to the front without duplicating it
        registrar_preparado(&cfg, p.uid, &repo, id, &cs[1], "0.3.1", "sha256:x").unwrap();
        let lista: Vec<String> = verificados(&cfg, p.uid)[id].preparados.iter().map(|x| x.0.clone()).collect();
        assert_eq!(lista, vec![cs[1].clone(), cs[3].clone(), cs[2].clone()]);
        std::env::remove_var("KALANGO_FLATPAK");
    }

    /// A stand-in flatpak whose `run` is an MCP server: `espera` seconds before it answers the handshake; with `trava`, it
    /// never answers a tool call.
    fn flatpak_mcp(d: &Path, espera: u32, trava: bool) {
        let fake = d.join("flatpak");
        let call =
            if trava { "exec sleep 300".to_string() } else { r#"printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[]}}\n' "$id""#.to_string() };
        std::fs::write(
            &fake,
            format!(
                r#"#!/bin/sh
[ "$1" = run ] || exit 0
sleep {espera}
while IFS= read -r l; do
  id=$(printf '%s' "$l" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$l" in
    *'"initialize"'*) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"protocolVersion":"2025-06-18"}}}}\n' "$id";;
    *'"tools/list"'*) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"tools":[{{"name":"doc_inspect"}}]}}}}\n' "$id";;
    *'"tools/call"'*) {call};;
  esac
done
"#
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        std::env::set_var("KALANGO_FLATPAK", &fake);
    }

    #[test]
    fn closing_the_app_while_its_server_starts_keeps_that_session_out() {
        let _g = AMBIENTE.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        let casa = d.path().join("casa");
        std::fs::create_dir_all(casa.join("Videos")).unwrap();
        flatpak_mcp(d.path(), 1, false);
        let (cfg, mut p) = pessoa(&casa, &d.path().join("estado"));
        // (a uid of its own: the table of sessions is shared by the tests that run at once)
        p.uid = 4203;
        let app = catalogo().app("filmcraft").unwrap();
        let rodou = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|sc| {
            let h = sc.spawn(|| {
                com_sessao(&cfg, &p, app, false, |_| {
                    rodou.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                })
            });
            std::thread::sleep(Duration::from_millis(400));
            // closed (the generation changes) while agents stay allowed in the settings: the new session still does not enter
            fechar_do_app(p.uid, &app.id);
            assert!(h.join().unwrap().is_err());
        });
        assert!(!rodou.load(std::sync::atomic::Ordering::SeqCst));
        assert!(tabela().as_ref().unwrap().mapa.is_empty());
        std::env::remove_var("KALANGO_FLATPAK");
    }

    #[test]
    fn a_call_waiting_behind_a_session_that_was_killed_does_not_run() {
        let _g = AMBIENTE.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        let casa = d.path().join("casa");
        std::fs::create_dir_all(casa.join("Videos")).unwrap();
        flatpak_mcp(d.path(), 0, true);
        let (cfg, mut p) = pessoa(&casa, &d.path().join("estado"));
        // (a uid of its own: the table of sessions is shared by the tests that run at once)
        p.uid = 4204;
        let app = catalogo().app("filmcraft").unwrap();
        let rodou = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|sc| {
            let h1 = sc.spawn(|| com_sessao(&cfg, &p, app, false, |s| s.chamar("doc_inspect", json!({}), PRAZO)));
            std::thread::sleep(Duration::from_millis(1500));
            // a second call waits for the first one's session; the session is killed from outside (not closed by the app,
            // so neither the generation nor the settings change)
            let h2 = sc.spawn(|| {
                com_sessao(&cfg, &p, app, false, |_| {
                    rodou.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                })
            });
            std::thread::sleep(Duration::from_millis(500));
            let ctl = tabela().as_ref().unwrap().mapa.values().next().unwrap().controle.clone();
            assert!(ctl.encerrar());
            assert!(h1.join().unwrap().is_err());
            let r2 = h2.join().unwrap();
            assert!(
                r2.is_err(),
                "the waiting call gets an error: {r2:?} ran={} entries={}",
                rodou.load(std::sync::atomic::Ordering::SeqCst),
                tabela().as_ref().unwrap().mapa.len()
            );
        });
        assert!(!rodou.load(std::sync::atomic::Ordering::SeqCst), "the waiting call never ran on the dead session");
        std::env::remove_var("KALANGO_FLATPAK");
    }

    #[test]
    fn the_flatpak_runs_with_a_pinned_environment_and_the_run_denies_sensitive_folders() {
        let _g = AMBIENTE.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        let saida = d.path().join("env.txt");
        let fake = d.path().join("flatpak");
        std::fs::write(&fake, format!("#!/bin/sh\nenv > {}\n", saida.display())).unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        std::env::set_var("KALANGO_FLATPAK", &fake);
        // what the environment of kalangod could carry (or a test of ours): none of it reaches the flatpak
        std::env::set_var("FLATPAK_USER_DIR", "/tmp/evil-flatpak");
        std::env::set_var("XDG_DATA_HOME", "/tmp/evil-data");
        std::env::set_var("FLATPAK_CONFIG_DIR", "/tmp/evil-config");
        let (_cfg, p) = pessoa(d.path(), &d.path().join("estado"));
        // (peer.home differs from the passwd home here, but no test installation is set: the real one is pinned)
        let st = flatpak_como(&p).arg("x").status().unwrap();
        for k in ["FLATPAK_USER_DIR", "XDG_DATA_HOME", "FLATPAK_CONFIG_DIR"] {
            std::env::remove_var(k);
        }
        std::env::remove_var("KALANGO_FLATPAK");
        assert!(st.success());
        let env = std::fs::read_to_string(&saida).unwrap();
        let casa = p.passwd_home.display();
        assert!(env.contains(&format!("FLATPAK_USER_DIR={casa}/.local/share/flatpak")), "{env}");
        assert!(env.contains(&format!("XDG_DATA_HOME={casa}/.local/share")) && env.contains("FLATPAK_CONFIG_DIR=/etc/flatpak"), "{env}");
        assert!(!env.contains("evil"), "{env}");
        // the server run carries the sensitive folders' denials after whatever the global override says
        let app = catalogo().app("filmcraft").unwrap();
        for janela in [None, Some(23456)] {
            let a = argumentos_do_servidor(app, &[PathBuf::from("/home/p/Vídeos")], janela);
            for x in ["~/.ssh", "~/.gnupg", "~/.aws", "~/.config/gcloud", "~/.kube", "~/.local/share/keyrings", "~/.mozilla"] {
                assert!(a.contains(&format!("--nofilesystem={x}")), "{x} {janela:?}");
            }
        }
    }
}
