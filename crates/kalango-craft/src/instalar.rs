//! Instalar, atualizar, voltar e remover um app criativo.
//!
//! Dois lados. O protegido (o kalangod, `preparar`, `importar_em`, `guardar_pontos`): verifica a imagem
//! (`verificar`), baixa as camadas conferidas e as importa num repositório do Flatpak onde só ele escreve
//! (`flatpak build-import-bundle --oci`), conferindo que os metadados do commit importado são os da configuração
//! assinada, e guardando
//! qual commit verificou e os pontos de restauração (as duas versões anteriores, com referências próprias no
//! repositório, por pessoa). O da pessoa (a Central, `instalar_commit`): instala desse
//! repositório, na instalação dela (`flatpak --user`), exatamente o commit que o kalangod devolveu, e confere depois.
//! Nada que a pessoa (ou um programa dela) escreva escolhe o que é instalado. O runtime (org.freedesktop.Platform) vem
//! do Flathub, que o Flatpak verifica com a chave dele. Permissões: rede desligada, sem o serviço do Flatpak no
//! computador e só as pastas conferidas (`flatpak override`, feito pelo kalangod).

use crate::catalogo::App;
use crate::verificar::{arquitetura, Verificada};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const REMOTO: &str = "kalango-apps-criativos";
/// Quantos pontos de restauração (versões anteriores) ficam por app.
pub const PONTOS: usize = 2;

pub fn flatpak() -> String {
    std::env::var("KALANGO_FLATPAK").unwrap_or_else(|_| "flatpak".into())
}

/// A pasta dos Apps criativos: o repositório local, os downloads e o estado.
pub fn pasta() -> PathBuf {
    if let Some(p) = std::env::var_os("KALANGO_CRAFT_DIR") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("kalango/apps-criativos")
}

/// O que a Central lembra de cada app instalado por ela.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Instalado {
    pub commit: String,
    pub versao: String,
    pub digest: String,
    /// segundos desde 1970
    pub quando: u64,
    /// os pontos de restauração: (commit, versão), o mais novo primeiro
    #[serde(default)]
    pub anteriores: Vec<(String, String)>,
    /// as atualizações feitas: (de, para, quando)
    #[serde(default)]
    pub historico: Vec<(String, String, u64)>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Estado {
    #[serde(default)]
    pub apps: BTreeMap<String, Instalado>,
    /// a última procura por versões novas
    #[serde(default)]
    pub verificado_em: u64,
}

pub fn agora() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn ler_estado() -> Estado {
    std::fs::read_to_string(pasta().join("estado.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn gravar_estado(e: &Estado) -> Result<(), String> {
    let p = pasta().join("estado.json");
    std::fs::create_dir_all(pasta()).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(e).unwrap_or_default()).map_err(|e| e.to_string())?;
    std::fs::rename(tmp, p).map_err(|e| e.to_string())
}

/// Roda o flatpak e devolve a saída; o erro é a última linha que ele escreveu.
pub fn rodar(args: &[&str]) -> Result<String, String> {
    let o =
        Command::new(flatpak()).args(args).env("LC_ALL", "C").env("LANG", "C").stdin(Stdio::null()).output().map_err(|e| format!("flatpak: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&o.stderr);
        Err(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("flatpak falhou").trim().to_string())
    }
}

/// O commit instalado de um app (na instalação da pessoa), se instalado.
pub fn commit_instalado(id: &str) -> Option<String> {
    rodar(&["--user", "info", "--show-commit", id]).ok().map(|s| s.trim().to_string()).filter(|s| s.len() == 64)
}

/// Os apps do catálogo instalados na instalação da pessoa.
pub fn instalados() -> Vec<String> {
    rodar(&["--user", "list", "--app", "--columns=application"])
        .map(|s| s.lines().map(|l| l.trim().to_string()).filter(|l| l.starts_with("ai.storyteller.")).collect())
        .unwrap_or_default()
}

/// Os apps abertos agora (uma atualização espera o app fechar).
pub fn em_execucao() -> Vec<String> {
    rodar(&["ps", "--columns=application"]).map(|s| s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()).unwrap_or_default()
}

/// Espaço livre (bytes) onde a pasta fica.
pub fn espaco_livre(p: &Path) -> Option<u64> {
    let mut q = p.to_path_buf();
    while !q.exists() {
        q = q.parent()?.to_path_buf();
    }
    let c = std::ffi::CString::new(q.as_os_str().as_encoded_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    Some(s.f_bavail as u64 * s.f_frsize as u64)
}

// ------------------------------------------------------------------------------------------------ the protected side
// (roda no kalangod: o repositório e a lista dos commits verificados ficam onde só ele escreve)

/// Um repositório ostree vazio (archive), legível por todos e escrito só por quem o cria.
pub fn preparar_repo(repo: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(repo).map_err(|e| e.to_string())?;
    let _ = std::fs::set_permissions(repo, std::fs::Permissions::from_mode(0o755));
    if !repo.join("config").is_file() {
        for d in ["objects", "refs/heads", "refs/remotes", "tmp", "state", "extensions"] {
            std::fs::create_dir_all(repo.join(d)).map_err(|e| e.to_string())?;
        }
        std::fs::write(repo.join("config"), "[core]\nrepo_version=1\nmode=archive-z2\n").map_err(|e| e.to_string())?;
        rodar(&["build-update-repo", repo.to_str().unwrap_or_default()])?;
    }
    Ok(())
}

fn ref_em(repo: &Path, referencia: &str) -> Option<String> {
    std::fs::read_to_string(repo.join("refs/heads").join(referencia)).ok().map(|s| s.trim().to_string()).filter(|s| s.len() == 64)
}

/// Só uma alteração do repositório protegido por vez (importar, guardar referências): dois preparos de pessoas ou apps
/// diferentes escrevem a mesma referência `app/<id>/<arch>/stable` e leem de volta o commit dela.
static REPO: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Importa a imagem verificada (o layout OCI em `oci`) no repositório; devolve o commit. O `build-import-bundle` faz um
/// commit novo a partir das camadas (o `org.flatpak.commit` da configuração é o da construção original, e o commit
/// importado é outro, com a mesma data): o que se confere, depois de importar, é que os metadados do commit que ficou no
/// repositório (as permissões que o app pede) são os da configuração assinada. Se não forem, a referência volta ao que
/// era e a importação é recusada.
pub fn importar_em(repo: &Path, v: &Verificada, oci: &Path) -> Result<String, String> {
    let _g = REPO.lock().unwrap_or_else(|e| e.into_inner());
    preparar_repo(repo)?;
    let antes = ref_em(repo, &v.referencia);
    rodar(&[
        "build-import-bundle",
        "--oci",
        &format!("--ref={}", v.referencia),
        repo.to_str().unwrap_or_default(),
        oci.to_str().unwrap_or_default(),
    ])?;
    let commit = ref_em(repo, &v.referencia).ok_or("o repositório ficou sem o app")?;
    rodar(&["build-update-repo", repo.to_str().unwrap_or_default()])?;
    if let Err(e) = conferir_metadados(repo, v) {
        let f = repo.join("refs/heads").join(&v.referencia);
        match antes {
            Some(c) => {
                let _ = std::fs::write(&f, format!("{c}\n"));
            }
            None => {
                let _ = std::fs::remove_file(&f);
            }
        }
        let _ = rodar(&["build-update-repo", repo.to_str().unwrap_or_default()]);
        return Err(e);
    }
    Ok(commit)
}

/// Os metadados de um keyfile, sem espaços nas pontas das linhas nem linhas vazias: duas escritas do mesmo conteúdo
/// ficam iguais.
fn normalizar_metadados(t: &str) -> String {
    t.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

/// Os metadados do commit importado (lidos por um remoto temporário do próprio repositório, numa instalação do Flatpak
/// só desta conferência) são os da configuração assinada (`org.flatpak.metadata`).
fn conferir_metadados(repo: &Path, v: &Verificada) -> Result<(), String> {
    let c: serde_json::Value = serde_json::from_slice(&v.config_bytes).map_err(|e| e.to_string())?;
    let assinados = c["config"]["Labels"]["org.flatpak.metadata"]
        .as_str()
        .filter(|m| !m.trim().is_empty())
        .ok_or("a configuração assinada não traz os metadados do app")?;
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let rodar_la = |args: &[&str]| -> Result<String, String> {
        let o = Command::new(flatpak())
            .args(args)
            .env("FLATPAK_USER_DIR", tmp.path())
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("flatpak: {e}"))?;
        if o.status.success() {
            Ok(String::from_utf8_lossy(&o.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&o.stderr).lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("flatpak falhou").trim().to_string())
        }
    };
    rodar_la(&["--user", "remote-add", "--no-gpg-verify", "conferencia", &format!("file://{}", repo.display())])?;
    let importados = rodar_la(&["--user", "remote-info", "--show-metadata", "conferencia", &v.referencia])?;
    if normalizar_metadados(&importados) != normalizar_metadados(assinados) {
        return Err("os metadados do commit importado não são os da configuração assinada".into());
    }
    Ok(())
}

/// Mantém no repositório os commits de `grupo` (pontos de restauração, preparados) de `id` para a pessoa `uid`, em
/// referências próprias (o prune não os leva). Cada pessoa e cada app tem a sua pasta: guardar uns nunca apaga os de
/// outra pessoa nem os de outro app.
pub fn guardar_refs(repo: &Path, grupo: &str, uid: u32, id: &str, commits: &[String]) -> Result<(), String> {
    if id.contains('/') || id.contains("..") || grupo.contains('/') {
        return Err("id inválido".into());
    }
    let _g = REPO.lock().unwrap_or_else(|e| e.into_inner());
    let dir = repo.join("refs/heads").join(grupo).join(uid.to_string()).join(id);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (n, c) in commits.iter().enumerate() {
        std::fs::write(dir.join(n.to_string()), format!("{c}\n")).map_err(|e| e.to_string())?;
    }
    rodar(&["build-update-repo", repo.to_str().unwrap_or_default()]).map(|_| ())
}

/// Os pontos de restauração de `id` para `uid` (as duas versões anteriores).
pub fn guardar_pontos(repo: &Path, uid: u32, id: &str, pontos: &[(String, String)]) -> Result<(), String> {
    let c: Vec<String> = pontos.iter().take(PONTOS).map(|p| p.0.clone()).collect();
    guardar_refs(repo, "kalango-anterior", uid, id, &c)
}

/// Os commits preparados e ainda não instalados de `id` para `uid`.
pub fn guardar_preparados(repo: &Path, uid: u32, id: &str, preparados: &[(String, String, String)]) -> Result<(), String> {
    let c: Vec<String> = preparados.iter().map(|p| p.0.clone()).collect();
    guardar_refs(repo, "kalango-preparado", uid, id, &c)
}

/// Quantos preparados (verificados, importados, ainda não instalados) ficam por pessoa e app: um segundo "Procurar
/// agora" durante uma atualização pendente não derruba o primeiro.
pub const PREPARADOS: usize = 3;

/// O que o lado protegido sabe de cada app de uma pessoa: o commit verificado em uso, os preparados (verificados e
/// importados, ainda não instalados) e os pontos de restauração — os únicos commits que a pessoa pode instalar.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Verificado {
    #[serde(default)]
    pub atual: Option<(String, String)>,
    #[serde(default)]
    pub preparados: Vec<(String, String, String)>,
    #[serde(default)]
    pub anteriores: Vec<(String, String)>,
}

/// Verifica, baixa e importa a versão atual de `app` no repositório protegido. `instalada`: a versão verificada em
/// uso (não se volta atrás). Devolve (commit, versão, digest).
pub fn preparar(
    cat: &crate::Catalogo,
    app: &App,
    repo: &Path,
    baixando: &Path,
    instalada: Option<&str>,
    a: &crate::verificar::Andamento,
) -> Result<(String, String, String), Erro> {
    let idx = crate::verificar::indice(cat).map_err(Erro::Verificacao)?;
    let anuncio = idx
        .into_iter()
        .find(|x| x.0 == app.id)
        .map(|x| x.1)
        .ok_or_else(|| Erro::Verificacao(crate::verificar::Falha::Formato("o índice do projeto não tem este app".into())))?;
    let v = crate::verificar::verificar(cat, app, &anuncio, instalada, false).map_err(Erro::Verificacao)?;
    let precisa = espaco_necessario(anuncio.tamanho_download, anuncio.tamanho_instalado);
    if let Some(livre) = espaco_livre(repo) {
        if livre < precisa {
            return Err(Erro::SemEspaco { faltam: precisa - livre });
        }
    }
    let oci = baixando.join(app.imagem());
    let _ = std::fs::remove_dir_all(&oci);
    crate::verificar::baixar(cat, app, &v, &oci, a).map_err(Erro::Verificacao)?;
    let commit = importar_em(repo, &v, &oci).map_err(Erro::Flatpak);
    let _ = std::fs::remove_dir_all(&oci);
    Ok((commit?, v.versao, v.digest))
}

// ------------------------------------------------------------------------------------------------ the person's side

/// O remoto dos apps verificados (o repositório protegido, só leitura para a pessoa) e o Flathub, de onde vem o
/// runtime. Um remoto com outro endereço volta ao certo (e, mesmo que alguém o troque, só fica instalado o commit
/// que o kalangod verificou).
pub fn preparar_remotos(repo: &Path) -> Result<(), String> {
    let url = format!("file://{}", repo.display());
    let remotos = rodar(&["--user", "remotes", "--columns=name,url"]).unwrap_or_default();
    match remotos.lines().map(|l| l.split('\t').map(str::trim).collect::<Vec<_>>()).find(|c| c.first() == Some(&REMOTO)) {
        // (sem GPG do Flatpak de propósito: o repositório só tem o que o kalangod verificou com a chave fixada, e a
        // instalação é presa ao commit verificado; ver docs/APPS-CRIATIVOS.md)
        None => {
            rodar(&[
                "--user",
                "remote-add",
                "--if-not-exists",
                "--no-gpg-verify",
                "--title=Apps criativos (verificados pelo KalangoOS)",
                REMOTO,
                &url,
            ])?;
        }
        Some(c) if c.get(1) != Some(&url.as_str()) => {
            rodar(&["--user", "remote-modify", &format!("--url={url}"), REMOTO])?;
        }
        _ => {}
    }
    if !remotos.lines().any(|l| l.split('\t').next().map(str::trim) == Some("flathub")) {
        rodar(&["--user", "remote-add", "--if-not-exists", "flathub", "https://dl.flathub.org/repo/flathub.flatpakrepo"])?;
    }
    Ok(())
}

/// O que deu errado ao instalar ou atualizar, para a tela (o design tem um estado para cada).
#[derive(Debug, Clone, PartialEq)]
pub enum Erro {
    Verificacao(crate::verificar::Falha),
    SemEspaco { faltam: u64 },
    EmUso,
    Flatpak(String),
}

impl std::fmt::Display for Erro {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Erro::Verificacao(x) => write!(f, "{x}"),
            Erro::SemEspaco { faltam } => write!(f, "sem espaço: faltam {} MB", faltam / 1_000_000),
            Erro::EmUso => write!(f, "o app está aberto"),
            Erro::Flatpak(e) => write!(f, "flatpak: {e}"),
        }
    }
}

/// Precisa de quanto espaço (download + repositório + instalação, com folga).
pub fn espaco_necessario(download: u64, instalado: u64) -> u64 {
    download * 2 + instalado + 200_000_000
}

/// Instala (ou atualiza para, ou volta a) exatamente `commit` de `app`, do repositório protegido `repo`. Depois, o
/// commit instalado tem de ser esse; se não for, o app sai. Guarda o histórico (só para a tela).
pub fn instalar_commit(app: &App, repo: &Path, commit: &str, versao: &str, digest: &str) -> Result<Instalado, Erro> {
    if commit.len() != 64 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Erro::Verificacao(crate::verificar::Falha::Formato("commit inválido".into())));
    }
    preparar_remotos(repo).map_err(Erro::Flatpak)?;
    let referencia = format!("app/{}/{}/stable", app.id, arquitetura().1);
    let antes = commit_instalado(&app.id);
    let mut estado = ler_estado();
    let anterior = estado.apps.get(&app.id).cloned().unwrap_or_default();
    let fixo = format!("--commit={commit}");
    let r = match &antes {
        Some(c) if c == commit => Ok(String::new()),
        Some(_) if origem(&app.id).as_deref() == Some(REMOTO) => rodar(&["--user", "update", "--noninteractive", "-y", &fixo, &referencia]),
        Some(_) => rodar(&["--user", "install", "--reinstall", "--noninteractive", "-y", REMOTO, &referencia])
            .and_then(|_| rodar(&["--user", "update", "--noninteractive", "-y", &fixo, &referencia])),
        None => rodar(&["--user", "install", "--noninteractive", "-y", REMOTO, &referencia]).and_then(|_| {
            if commit_instalado(&app.id).as_deref() == Some(commit) {
                Ok(String::new())
            } else {
                rodar(&["--user", "update", "--noninteractive", "-y", &fixo, &referencia])
            }
        }),
    };
    r.map_err(Erro::Flatpak)?;
    // o que ficou instalado é o que o kalangod verificou
    let agora_c = commit_instalado(&app.id).ok_or_else(|| Erro::Flatpak("o app não ficou instalado".into()))?;
    if agora_c != commit {
        let _ = rodar(&["--user", "uninstall", "--noninteractive", "-y", &app.id]);
        return Err(Erro::Verificacao(crate::verificar::Falha::Assinatura("o commit instalado não é o verificado".into())));
    }
    let mut novo = Instalado {
        commit: commit.into(),
        versao: versao.into(),
        digest: digest.into(),
        quando: agora(),
        anteriores: Vec::new(),
        historico: anterior.historico.clone(),
    };
    if antes.is_some() && !anterior.versao.is_empty() && anterior.versao != versao {
        novo.historico.insert(0, (anterior.versao.clone(), versao.into(), agora()));
        novo.historico.truncate(10);
    }
    estado.apps.insert(app.id.clone(), novo.clone());
    gravar_estado(&estado).map_err(Erro::Flatpak)?;
    Ok(novo)
}

/// A origem (remoto) de um app instalado.
pub fn origem(id: &str) -> Option<String> {
    rodar(&["--user", "info", "--show-origin", id]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Remove o app (e os dados dele em ~/.var/app, se `dados`). Os projetos nas pastas da pessoa nunca são apagados.
pub fn remover(app: &App, dados: bool) -> Result<(), String> {
    let mut a = vec!["--user", "uninstall", "--noninteractive", "-y"];
    if dados {
        a.push("--delete-data");
    }
    a.push(&app.id);
    rodar(&a)?;
    let mut e = ler_estado();
    e.apps.remove(&app.id);
    gravar_estado(&e)?;
    let _ = rodar(&["override", "--user", "--reset", &app.id]);
    Ok(())
}

/// Sockets do Flatpak que dão acesso amplo ao computador da pessoa (o barramento de sessão e o de sistema sem filtro, o
/// agente de chaves SSH e o do GPG, o leitor de cartões e a impressão): nenhum app criativo os tem. A tela, o som e a
/// GPU (`dri`) ficam: a Central abre a janela do app. Sem a janela, o kalangod tira também a tela e o som.
pub const SOCKETS_NEGADOS: &[&str] = &["session-bus", "system-bus", "ssh-auth", "gpg-agent", "pcsc", "cups"];
/// Dispositivos negados: todos (`all`), o KVM e o /dev/shm do computador; só `dri`, se o app o pede.
pub const DISPOSITIVOS_NEGADOS: &[&str] = &["all", "kvm", "shm"];
/// Funções que alargam o sandbox (ferramentas de desenvolvimento, outra arquitetura, Bluetooth, CAN).
pub const FUNCOES_NEGADAS: &[&str] = &["devel", "multiarch", "bluetooth", "canbus"];
/// Nomes de barramento que nenhum app criativo fala, peça-os ou não: o serviço do Flatpak (roda programas fora da
/// caixa), os chaveiros, o systemd e o login do computador.
pub const NOMES_NEGADOS: &[&str] = &[
    "org.freedesktop.Flatpak",
    "org.freedesktop.secrets",
    "org.gnome.keyring",
    "org.freedesktop.systemd1",
    "org.freedesktop.login1",
    "org.freedesktop.PolicyKit1",
];
/// Os únicos nomes que um app pode continuar falando, se pedir.
pub const NOMES_PERMITIDOS: &[&str] = &["org.freedesktop.Notifications"];

/// Pastas e arquivos da pessoa que nenhum app criativo vê, mesmo que um override global (que vale por baixo do override do
/// app e dos argumentos do `flatpak run`) tente abri-los: chaves, nuvens, perfis de navegadores, dados de outros apps. Dita
/// em cada `flatpak run` e no override do app, que vêm depois do global: o que o global abrir de específico aqui é negado.
pub const SENSIVEIS: &[&str] = &[
    "~/.ssh",
    "~/.gnupg",
    "~/.aws",
    "~/.config/gcloud",
    "~/.azure",
    "~/.kube",
    "~/.docker",
    "~/.netrc",
    "~/.local/share/keyrings",
    "~/.password-store",
    "~/.mozilla",
    "~/.config/google-chrome",
    "~/.config/chromium",
    "~/.config/BraveSoftware",
    "~/.config/microsoft-edge",
    "~/.config/vivaldi",
    "~/snap",
    "~/.var/app",
    "~/.config/flatpak",
    "~/.local/share/flatpak/overrides",
];

/// As negações que não dependem do app, no formato do `flatpak override` e do `flatpak run`.
pub fn negacoes_fixas() -> Vec<String> {
    let mut a = Vec::new();
    a.extend(SENSIVEIS.iter().map(|x| format!("--nofilesystem={x}")));
    a.extend(SOCKETS_NEGADOS.iter().map(|x| format!("--nosocket={x}")));
    a.extend(DISPOSITIVOS_NEGADOS.iter().map(|x| format!("--nodevice={x}")));
    a.extend(FUNCOES_NEGADAS.iter().map(|x| format!("--disallow={x}")));
    a.extend(NOMES_NEGADOS.iter().map(|x| format!("--no-talk-name={x}")));
    a
}

/// O que o app pede nos metadados: pastas (`filesystems=`) e nomes de barramento (`[Session Bus Policy]`,
/// `[System Bus Policy]`); o que pedir é negado, menos o permitido.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pedidos {
    pub pastas: Vec<String>,
    pub nomes: Vec<String>,
    pub nomes_do_sistema: Vec<String>,
}

impl Pedidos {
    pub fn de(metadados: &str) -> Pedidos {
        let nome_ok =
            |n: &str| !n.is_empty() && !n.starts_with('-') && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '*'));
        let mut p = Pedidos { pastas: pastas_pedidas(metadados), ..Default::default() };
        let mut secao = "";
        for l in metadados.lines() {
            let l = l.trim();
            if l.starts_with('[') {
                secao = l;
                continue;
            }
            let Some((n, _)) = l.split_once('=') else { continue };
            let n = n.trim();
            if !nome_ok(n) {
                continue;
            }
            match secao {
                "[Session Bus Policy]" => p.nomes.push(n.to_string()),
                "[System Bus Policy]" => p.nomes_do_sistema.push(n.to_string()),
                _ => {}
            }
        }
        p
    }
}

/// O que o override **global** da pessoa (`flatpak override --user`, sem app: `~/.local/share/flatpak/overrides/global`)
/// põe a mais para todos os apps. O `--reset` de um app só apaga o override dele; o global vale por baixo dele e por cima
/// dos metadados, e um programa da pessoa pode tê-lo escrito (`filesystems=~/.ssh`). Devolve o primeiro item que alarga o
/// que um app criativo vê (`None` se o global só nega, ou só traz o que o app já tem: tela, som, GPU, IPC). Um item de
/// que não se sabe o efeito alarga.
pub fn override_global_alarga(texto: &str) -> Option<String> {
    let mut secao = "";
    for l in texto.lines() {
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if l.starts_with('[') {
            secao = l;
            if !["[Context]", "[Session Bus Policy]", "[System Bus Policy]"].contains(&l) {
                return Some(l.to_string());
            }
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { return Some(l.to_string()) };
        let (k, v) = (k.trim(), v.trim());
        let itens = || v.split(';').map(str::trim).filter(|x| !x.is_empty());
        let ok = |permitidos: &[&str]| itens().all(|x| x.starts_with('!') || permitidos.contains(&x));
        let bom = match (secao, k) {
            ("[Context]", "filesystems" | "persistent" | "features" | "unset-environment") => {
                k == "unset-environment" || itens().all(|x| x.starts_with('!'))
            }
            ("[Context]", "sockets") => ok(&["x11", "wayland", "fallback-x11", "pulseaudio"]),
            ("[Context]", "devices") => ok(&["dri"]),
            ("[Context]", "shared") => ok(&["ipc"]),
            ("[Session Bus Policy]" | "[System Bus Policy]", _) => v == "none" || v == "see",
            _ => false,
        };
        if !bom {
            return Some(format!("{k}={v}"));
        }
    }
    None
}

/// As permissões do app no Flatpak: rede, as pastas que ele vê e nada que dê acesso amplo ao computador (o serviço do
/// Flatpak, o barramento sem filtro, as chaves, os dispositivos, as funções que alargam o sandbox, os nomes de
/// barramento que ele pede). `pedidos` é o que o app pede nos metadados; só as pastas escolhidas ficam.
pub fn argumentos_de_permissao(id: &str, rede: bool, pastas: &[String], pedidos: &Pedidos) -> Vec<String> {
    let mut a = vec!["override".to_string(), "--user".to_string()];
    a.push(if rede { "--share=network" } else { "--unshare=network" }.into());
    // (o app pede org.freedesktop.Flatpak para rodar programas fora da caixa — o atualizador dele: aqui quem atualiza é a
    // Central)
    a.extend(negacoes_fixas());
    for n in &pedidos.nomes {
        if !NOMES_PERMITIDOS.contains(&n.as_str()) && !NOMES_NEGADOS.contains(&n.as_str()) {
            a.push(format!("--no-talk-name={n}"));
        }
    }
    for n in &pedidos.nomes_do_sistema {
        a.push(format!("--system-no-talk-name={n}"));
    }
    for amplo in ["host", "host-os", "host-etc", "home", "/media", "/run/media", "/mnt", "xdg-data/ArtCraft/versions"] {
        a.push(format!("--nofilesystem={amplo}"));
    }
    for p in &pedidos.pastas {
        let base = p.split(':').next().unwrap_or(p);
        if !pastas.iter().any(|x| x == base) {
            a.push(format!("--nofilesystem={base}"));
        }
    }
    for p in pastas {
        a.push(format!("--filesystem={p}"));
    }
    a.push(id.to_string());
    a
}

/// O que o app pede de pastas (`filesystems=` do `[Context]` dos metadados).
pub fn pastas_pedidas(metadados: &str) -> Vec<String> {
    let mut dentro = false;
    for l in metadados.lines() {
        let l = l.trim();
        if l.starts_with('[') {
            dentro = l == "[Context]";
            continue;
        }
        if dentro {
            if let Some(v) = l.strip_prefix("filesystems=") {
                return v.split(';').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
            }
        }
    }
    Vec::new()
}

/// Aplica as permissões a um app instalado.
pub fn aplicar_permissoes(id: &str, rede: bool, pastas: &[String]) -> Result<(), String> {
    let meta = rodar(&["--user", "info", "--show-metadata", id]).unwrap_or_default();
    let pedidos = Pedidos::de(&meta);
    rodar(&["override", "--user", "--reset", id])?;
    let a = argumentos_de_permissao(id, rede, pastas, &pedidos);
    let r: Vec<&str> = a.iter().map(String::as_str).collect();
    rodar(&r).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_turn_off_the_network_the_host_and_broad_folders() {
        let ped = Pedidos {
            pastas: vec!["home".into(), "/media".into(), "xdg-videos".into(), "xdg-data/ArtCraft/versions:ro".into()],
            ..Default::default()
        };
        let a = argumentos_de_permissao("ai.storyteller.filmcraft", false, &["xdg-videos".into()], &ped);
        assert!(a.contains(&"--unshare=network".to_string()));
        assert!(a.contains(&"--no-talk-name=org.freedesktop.Flatpak".to_string()));
        assert!(a.contains(&"--nofilesystem=home".to_string()));
        assert!(a.contains(&"--nofilesystem=host".to_string()));
        assert!(a.contains(&"--filesystem=xdg-videos".to_string()));
        assert!(!a.contains(&"--nofilesystem=xdg-videos".to_string()));
        assert_eq!(a.last().unwrap(), "ai.storyteller.filmcraft");
        let b = argumentos_de_permissao("x", true, &[], &Pedidos { pastas: vec!["xdg-pictures".into()], ..Default::default() });
        assert!(b.contains(&"--share=network".to_string()));
        assert!(b.contains(&"--nofilesystem=xdg-pictures".to_string()));
    }

    #[test]
    fn requested_folders_come_from_the_context_section() {
        let m = "[Application]\nname=x\n\n[Context]\nshared=network;ipc;\nfilesystems=home;/media;xdg-data/ArtCraft/versions:ro;\n\n[Session Bus Policy]\norg.freedesktop.Flatpak=talk\n";
        assert_eq!(pastas_pedidas(m), vec!["home", "/media", "xdg-data/ArtCraft/versions:ro"]);
    }

    #[test]
    fn space_needed_counts_download_repo_and_install() {
        assert!(espaco_necessario(100_000_000, 200_000_000) >= 400_000_000);
    }

    /// (tests that point KALANGO_FLATPAK at a stand-in run one at a time)
    static AMBIENTE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// A stand-in `flatpak`: `build-import-bundle` writes the commit named in the file `$OCI/commit` into the ref of the
    /// repository; `remote-info --show-metadata` prints `$OCI/metadata` (what the imported commit carries, here taken
    /// from the folder named in $KALANGO_TESTE_OCI); everything else succeeds.
    fn falso(d: &Path) -> PathBuf {
        let f = d.join("flatpak");
        std::fs::write(
            &f,
            r#"#!/bin/sh
case "$*" in
  *remote-info*) cat "$KALANGO_TESTE_OCI/metadata"; exit 0;;
esac
if [ "$1" = build-import-bundle ]; then
  ref=${3#--ref=}; repo=$4; oci=$5
  mkdir -p "$repo/refs/heads/$(dirname "$ref")"
  cp "$oci/commit" "$repo/refs/heads/$ref"
fi
exit 0
"#,
        )
        .unwrap();
        std::fs::set_permissions(&f, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        f
    }

    #[test]
    fn an_import_whose_metadata_is_not_the_signed_one_is_refused_and_the_ref_goes_back() {
        let _g = AMBIENTE.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        std::env::set_var("KALANGO_FLATPAK", falso(d.path()));
        let repo = d.path().join("repo");
        let oci = d.path().join("oci");
        std::fs::create_dir_all(&oci).unwrap();
        std::env::set_var("KALANGO_TESTE_OCI", &oci);
        let (velho, novo) = ("a".repeat(64), "b".repeat(64));
        let assinado = "[Application]\nname=x.y\n\n[Context]\nfilesystems=xdg-videos;\n";
        let config = serde_json::json!({"config": {"Labels": {"org.flatpak.metadata": assinado}}});
        let v = Verificada { referencia: "app/x.y/x86_64/stable".into(), config_bytes: config.to_string().into_bytes(), ..Default::default() };
        preparar_repo(&repo).ok();
        std::fs::create_dir_all(repo.join("refs/heads/app/x.y/x86_64")).unwrap();
        std::fs::write(repo.join("refs/heads/app/x.y/x86_64/stable"), format!("{velho}\n")).unwrap();
        std::fs::write(oci.join("commit"), format!("{novo}\n")).unwrap();
        // the imported commit asks for more than the signed config says (a wider filesystem): refused, the ref goes back
        std::fs::write(oci.join("metadata"), "[Application]\nname=x.y\n\n[Context]\nfilesystems=host;\n").unwrap();
        let e = importar_em(&repo, &v, &oci).unwrap_err();
        assert!(e.contains("não são os da configuração assinada"), "{e}");
        assert_eq!(ref_em(&repo, &v.referencia).as_deref(), Some(velho.as_str()), "the ref is back to what it was");
        // the same metadata (spacing aside): accepted
        std::fs::write(oci.join("metadata"), format!("  {assinado}\n\n")).unwrap();
        assert_eq!(importar_em(&repo, &v, &oci).unwrap(), novo);
        // a signed config without the metadata label: refused (fail closed)
        let sem = Verificada { config_bytes: b"{}".to_vec(), ..v.clone() };
        assert!(importar_em(&repo, &sem, &oci).is_err());
        std::env::remove_var("KALANGO_FLATPAK");
        std::env::remove_var("KALANGO_TESTE_OCI");
    }

    #[test]
    fn restore_points_are_kept_per_person_and_per_app() {
        let _g = AMBIENTE.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        std::env::set_var("KALANGO_FLATPAK", falso(d.path()));
        let repo = d.path().join("repo");
        let (a, b) = ("a".repeat(64), "b".repeat(64));
        guardar_pontos(&repo, 1000, "x.y", &[(a.clone(), "1".into())]).unwrap();
        guardar_pontos(&repo, 1001, "x.y", &[(b.clone(), "2".into())]).unwrap();
        guardar_pontos(&repo, 1000, "z.w", &[(b.clone(), "2".into())]).unwrap();
        // another person's, then the same person again with an empty list: only that person's app is emptied
        guardar_pontos(&repo, 1000, "x.y", &[]).unwrap();
        let ler = |uid: u32, id: &str| std::fs::read_to_string(repo.join(format!("refs/heads/kalango-anterior/{uid}/{id}/0"))).ok();
        assert_eq!(ler(1001, "x.y").as_deref(), Some(format!("{b}\n").as_str()), "another person's point stays");
        assert_eq!(ler(1000, "z.w").as_deref(), Some(format!("{b}\n").as_str()), "another app's point stays");
        assert!(ler(1000, "x.y").is_none());
        assert!(guardar_pontos(&repo, 1000, "../x", &[]).is_err());
        std::env::remove_var("KALANGO_FLATPAK");
    }

    const META: &str = "[Application]\nname=x\n\n[Context]\nshared=network;ipc;\nsockets=x11;wayland;session-bus;ssh-auth;\ndevices=all;\nfeatures=devel;bluetooth;\nfilesystems=home;xdg-videos;\n\n[Session Bus Policy]\norg.freedesktop.Flatpak=talk\norg.freedesktop.Notifications=talk\norg.kde.*=own\nx.y.Z=own\n--evil=talk\n\n[System Bus Policy]\norg.bluez=talk\n";

    #[test]
    fn the_app_requests_are_read_and_unsafe_names_are_dropped() {
        let p = Pedidos::de(META);
        assert_eq!(p.pastas, vec!["home", "xdg-videos"]);
        assert_eq!(
            p.nomes,
            vec!["org.freedesktop.Flatpak", "org.freedesktop.Notifications", "org.kde.*", "x.y.Z"],
            "an option-looking name never becomes an argument"
        );
        assert_eq!(p.nomes_do_sistema, vec!["org.bluez"]);
    }

    /// The effective result with the real `flatpak` in a scratch installation (no network, no app installed): what
    /// `flatpak override --show` reports after the arguments are applied.
    #[test]
    fn the_effective_override_denies_the_wide_access_a_creative_app_asks_for() {
        if Command::new("flatpak").arg("--version").output().is_err() {
            assert!(
                std::env::var_os("KALANGO_TESTE_SEM_FLATPAK").is_some(),
                "flatpak is not installed: install it, or set KALANGO_TESTE_SEM_FLATPAK=1 to skip this test"
            );
            eprintln!("SKIPPED: no flatpak on this machine");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let sh = |args: &[String]| {
            let o = Command::new("flatpak").args(args).env("FLATPAK_USER_DIR", d.path()).env("LC_ALL", "C").output().unwrap();
            assert!(o.status.success(), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
            String::from_utf8_lossy(&o.stdout).into_owned()
        };
        let a = argumentos_de_permissao("ai.storyteller.filmcraft", false, &["xdg-videos".into()], &Pedidos::de(META));
        sh(&a);
        let o = sh(&["override".into(), "--user".into(), "--show".into(), "ai.storyteller.filmcraft".into()]);
        // (listas escritas aqui de propósito: as constantes do código não servem de gabarito de si mesmas)
        for socket in ["session-bus", "system-bus", "ssh-auth", "gpg-agent", "pcsc", "cups"] {
            assert!(o.contains(&format!("!{socket}")), "{socket} denied: {o}");
        }
        assert!(!o.contains("!x11") && !o.contains("!wayland"), "the window's screen is not touched: {o}");
        for x in ["!all", "!kvm", "!shm", "!devel", "!multiarch", "!bluetooth", "!canbus", "!network", "!home", "xdg-videos"] {
            assert!(o.contains(x), "{x}: {o}");
        }
        assert!(!o.contains("devices=dri") && !o.contains("!dri"), "{o}");
        for n in ["org.freedesktop.Flatpak=none", "org.freedesktop.secrets=none", "org.kde.*=none", "x.y.Z=none"] {
            assert!(o.contains(n), "{n}: {o}");
        }
        assert!(!o.contains("org.freedesktop.Notifications"), "an allowed name is left alone: {o}");
        assert!(o.contains("[System Bus Policy]") && o.contains("org.bluez=none"), "{o}");
    }

    #[test]
    fn a_global_override_that_only_denies_is_fine_and_one_that_widens_is_found() {
        assert_eq!(override_global_alarga(""), None);
        assert_eq!(
            override_global_alarga(
                "[Context]\nfilesystems=!host;!home;\nsockets=!cups;x11;\ndevices=dri;\nshared=ipc;\n\n[Session Bus Policy]\norg.a.B=none\n"
            ),
            None
        );
        for ruim in [
            "[Context]\nfilesystems=~/.ssh;\n",
            "[Context]\nfilesystems=!home;/etc:ro;\n",
            "[Context]\npersistent=.ssh;\n",
            "[Context]\nsockets=ssh-auth;\n",
            "[Context]\ndevices=all;\n",
            "[Context]\nfeatures=devel;\n",
            "[Context]\nshared=network;\n",
            "[Session Bus Policy]\norg.freedesktop.secrets=talk\n",
            "[System Bus Policy]\norg.a.B=own\n",
            "[Environment]\nLD_PRELOAD=/x.so\n",
            "[Context]\nnovo-campo=x\n",
            "lixo",
        ] {
            assert!(override_global_alarga(ruim).is_some(), "{ruim}");
        }
    }

    #[test]
    fn sensitive_folders_are_denied_in_every_run_and_override() {
        let a = negacoes_fixas();
        for x in [
            "~/.ssh",
            "~/.gnupg",
            "~/.aws",
            "~/.config/gcloud",
            "~/.kube",
            "~/.local/share/keyrings",
            "~/.mozilla",
            "~/.config/google-chrome",
            "~/.var/app",
        ] {
            assert!(a.contains(&format!("--nofilesystem={x}")), "{x}");
        }
        // (and the arguments of the override carry them too, after the global's)
        let o = argumentos_de_permissao("x.y", false, &[], &Pedidos::default());
        assert!(o.contains(&"--nofilesystem=~/.ssh".to_string()));
    }
}
