//! A cadeia que decide se uma imagem de um app entra no sistema:
//! 1. o índice OCI do projeto diz o digest da imagem atual de cada app (o índice em si não é assinado: só aponta);
//! 2. a assinatura dessa imagem (`<assinaturas>/<repo>@sha256=<hex>/signature-1`, o formato "atomic container
//!    signature" do containers/image) é conferida pelo `gpgv` com a chave do catálogo, e a chave primária que assinou
//!    tem de ser a da impressão digital fixada; o texto assinado tem de nomear exatamente este digest e
//!    `ghcr.io/<repo>`;
//! 3. o manifesto baixado do registro tem de ter esse sha256; a configuração e as camadas, os sha256 do manifesto;
//! 4. a configuração dá o ref, o commit da construção e os metadados do Flatpak. O `build-import-bundle` faz um commit
//!    novo das camadas (outro id, a mesma data), então o id não se compara; a instalação confere depois de importar que
//!    os metadados do commit importado (as permissões que o app pede) são os da configuração assinada, e só instala o
//!    commit que ela mesma importou e guardou.
//!
//! Qualquer elo que não bate recusa a instalação, antes de qualquer coisa ser escrita fora da pasta de download.

use crate::catalogo::{App, Catalogo, CHAVE};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// O erro de uma verificação: o que o design chama de "Assinatura não confere" é `Assinatura`.
#[derive(Debug, Clone, PartialEq)]
pub enum Falha {
    /// sem rede, tempo esgotado, servidor fora
    Rede(String),
    /// a assinatura falta, não é da chave fixada ou não cobre esta imagem; um digest que não bate
    Assinatura(String),
    /// a versão está na lista de bloqueio do catálogo
    Bloqueada {
        versao: String,
        motivo: String,
    },
    /// o que veio não tem a forma esperada
    Formato(String),
    Cancelada,
}

impl std::fmt::Display for Falha {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Falha::Rede(e) => write!(f, "rede: {e}"),
            Falha::Assinatura(e) => write!(f, "assinatura não confere: {e}"),
            Falha::Bloqueada { versao, motivo } => write!(f, "versão {versao} bloqueada pelo KalangoOS: {motivo}"),
            Falha::Formato(e) => write!(f, "formato inesperado: {e}"),
            Falha::Cancelada => write!(f, "cancelada"),
        }
    }
}

/// A imagem atual de um app, como o índice a anuncia (ainda não verificada).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Anuncio {
    pub digest: String,
    pub versao: String,
    pub tamanho_download: u64,
    pub tamanho_instalado: u64,
    pub runtime: String,
    /// o ícone de 128 px (PNG) que o projeto põe na imagem
    pub icone_png: Option<Vec<u8>>,
}

/// A imagem verificada: o que a instalação usa.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Verificada {
    pub digest: String,
    pub versao: String,
    pub referencia: String,
    pub commit: String,
    pub config: (String, u64),
    pub camadas: Vec<(String, u64)>,
    pub manifesto: Vec<u8>,
    pub config_bytes: Vec<u8>,
}

fn agente() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(30))
        .user_agent("KalangoOS-Central-de-Apps")
        .build()
}

fn texto(url: &str) -> Result<Vec<u8>, Falha> {
    let r = agente().get(url).call().map_err(|e| Falha::Rede(e.to_string()))?;
    let mut b = Vec::new();
    r.into_reader().take(64 << 20).read_to_end(&mut b).map_err(|e| Falha::Rede(e.to_string()))?;
    Ok(b)
}

pub fn sha256_hex(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

/// A arquitetura do índice ("amd64") e do Flatpak ("x86_64") desta máquina.
pub fn arquitetura() -> (&'static str, &'static str) {
    match std::env::consts::ARCH {
        "aarch64" => ("arm64", "aarch64"),
        _ => ("amd64", "x86_64"),
    }
}

/// A versão do primeiro `<release version=…>` de um appdata.
pub fn versao_do_appdata(appdata: &str) -> Option<String> {
    appdata
        .split("<release ")
        .nth(1)
        .and_then(|s| s.split("version=\"").nth(1))
        .and_then(|s| s.split('"').next())
        .map(str::to_string)
        .filter(|v| !v.is_empty())
}

/// Compara versões "0.20.1" número a número (o que não é número conta como 0).
pub fn comparar_versoes(a: &str, b: &str) -> std::cmp::Ordering {
    let n = |s: &str| s.split(['.', '-', '+']).map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (x, y) = (n(a), n(b));
    for i in 0..x.len().max(y.len()) {
        let o = x.get(i).copied().unwrap_or(0).cmp(&y.get(i).copied().unwrap_or(0));
        if o != std::cmp::Ordering::Equal {
            return o;
        }
    }
    std::cmp::Ordering::Equal
}

/// Lê o índice: para cada app do catálogo, a imagem desta arquitetura no canal do catálogo.
pub fn ler_indice(cat: &Catalogo, json: &[u8]) -> Result<Vec<(String, Anuncio)>, Falha> {
    let v: Value = serde_json::from_slice(json).map_err(|e| Falha::Formato(e.to_string()))?;
    let (arch, _) = arquitetura();
    let tag = format!("{}-{}", cat.fonte.canal, arquitetura().1);
    let mut out = Vec::new();
    for r in v["Results"].as_array().into_iter().flatten() {
        let nome = r["Name"].as_str().unwrap_or("");
        let Some(app) = cat.apps.iter().find(|a| a.repositorio == nome) else { continue };
        let Some(im) = r["Images"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|i| i["Architecture"] == arch && i["Tags"].as_array().is_some_and(|t| t.iter().any(|x| x == tag.as_str())))
        else {
            continue;
        };
        let l = &im["Labels"];
        let n = |k: &str| l[k].as_str().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let appdata = l["org.freedesktop.appstream.appdata"].as_str().unwrap_or("");
        // a versão: a do primeiro <release version=…> do appdata, senão a da etiqueta "0.20.1-x86_64"
        let versao = versao_do_appdata(appdata)
            .or_else(|| {
                im["Tags"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()))
                    .and_then(|t| t.rsplit_once('-'))
                    .map(|(v, _)| v.to_string())
            })
            .unwrap_or_default();
        let runtime = l["org.flatpak.metadata"].as_str().unwrap_or("").lines().find_map(|x| x.strip_prefix("runtime=")).unwrap_or("").to_string();
        let icone_png = l["org.freedesktop.appstream.icon-128"]
            .as_str()
            .and_then(|s| s.strip_prefix("data:image/png;base64,").or(Some(s)))
            .and_then(|b| base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b).ok());
        out.push((
            app.id.clone(),
            Anuncio {
                digest: im["Digest"].as_str().unwrap_or("").to_string(),
                versao,
                tamanho_download: n("org.flatpak.download-size"),
                tamanho_instalado: n("org.flatpak.installed-size"),
                runtime,
                icone_png,
            },
        ));
    }
    Ok(out)
}

/// Baixa e lê o índice do projeto.
pub fn indice(cat: &Catalogo) -> Result<Vec<(String, Anuncio)>, Falha> {
    let (arch, _) = arquitetura();
    let url =
        format!("{}?label%3Aorg.flatpak.ref%3Aexists=1&architecture={arch}&os=linux&tag={}-{}", cat.fonte.indice, cat.fonte.canal, arquitetura().1);
    ler_indice(cat, &texto(&url)?)
}

fn digest_ok(d: &str) -> bool {
    d.len() == 71 && d.starts_with("sha256:") && d[7..].bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// Confere uma assinatura com o `gpgv` e a chave do catálogo. Devolve o texto assinado.
pub fn conferir_assinatura(cat: &Catalogo, assinatura: &[u8]) -> Result<Vec<u8>, Falha> {
    // (uma pasta nova, 0700 e de nome imprevisível, que falha se já existir: nada de outro processo entra no meio)
    let tmp = tempfile::Builder::new().prefix("kalango-craft-gpgv-").tempdir().map_err(|e| Falha::Formato(e.to_string()))?;
    let dir = tmp.path().to_path_buf();
    let chave = dir.join("chave.gpg");
    let sig = dir.join("signature-1");
    let saida = dir.join("texto");
    let r = (|| {
        std::fs::write(&chave, CHAVE).map_err(|e| Falha::Formato(e.to_string()))?;
        std::fs::write(&sig, assinatura).map_err(|e| Falha::Formato(e.to_string()))?;
        let o = std::process::Command::new(gpgv())
            .args(["--status-fd", "1", "--keyring"])
            .arg(&chave)
            .arg("--output")
            .arg(&saida)
            .arg(&sig)
            .env("LANG", "C")
            .env("LC_ALL", "C")
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .map_err(|e| Falha::Assinatura(format!("gpgv: {e}")))?;
        let status = String::from_utf8_lossy(&o.stdout);
        // [GNUPG:] VALIDSIG <subchave> <data> <ts> <exp> <ver> <res> <pk> <hash> <classe> <primária>
        let primaria =
            status.lines().find_map(|l| l.strip_prefix("[GNUPG:] VALIDSIG ")).and_then(|r| r.split_whitespace().nth(9)).map(str::to_string);
        if !o.status.success() || primaria.is_none() {
            return Err(Falha::Assinatura("assinatura inválida ou de outra chave".into()));
        }
        if !primaria.as_deref().unwrap_or("").eq_ignore_ascii_case(&cat.fonte.impressao_digital) {
            return Err(Falha::Assinatura(format!("assinada por {} e não pela chave fixada", primaria.unwrap_or_default())));
        }
        std::fs::read(&saida).map_err(|e| Falha::Assinatura(e.to_string()))
    })();
    drop(tmp);
    r
}

fn gpgv() -> String {
    std::env::var("KALANGO_GPGV").unwrap_or_else(|_| "gpgv".into())
}

/// O texto assinado tem de cobrir exatamente esta imagem, deste repositório.
pub fn conferir_texto_assinado(texto: &[u8], registro: &str, repositorio: &str, digest: &str) -> Result<(), Falha> {
    let v: Value = serde_json::from_slice(texto).map_err(|_| Falha::Assinatura("o texto assinado não é JSON".into()))?;
    let c = &v["critical"];
    if c["type"] != "atomic container signature" {
        return Err(Falha::Assinatura("tipo de assinatura inesperado".into()));
    }
    if c["image"]["docker-manifest-digest"].as_str() != Some(digest) {
        return Err(Falha::Assinatura("a assinatura é de outra imagem".into()));
    }
    if c["identity"]["docker-reference"].as_str() != Some(&format!("{registro}/{repositorio}")) {
        return Err(Falha::Assinatura("a assinatura é de outro repositório".into()));
    }
    Ok(())
}

/// Uma ficha anônima de leitura do registro (o ghcr.io pede uma mesmo para imagens públicas).
fn ficha(cat: &Catalogo, repo: &str) -> Result<String, Falha> {
    let b = texto(&format!("https://{}/token?scope=repository:{repo}:pull", cat.fonte.registro))?;
    let v: Value = serde_json::from_slice(&b).map_err(|e| Falha::Formato(e.to_string()))?;
    v["token"].as_str().map(str::to_string).ok_or_else(|| Falha::Formato("sem ficha do registro".into()))
}

fn do_registro(cat: &Catalogo, repo: &str, ficha: &str, caminho: &str, aceita: &str) -> Result<Vec<u8>, Falha> {
    let r = agente()
        .get(&format!("https://{}/v2/{repo}/{caminho}", cat.fonte.registro))
        .set("Authorization", &format!("Bearer {ficha}"))
        .set("Accept", aceita)
        .call()
        .map_err(|e| Falha::Rede(e.to_string()))?;
    let mut b = Vec::new();
    r.into_reader().take(8 << 20).read_to_end(&mut b).map_err(|e| Falha::Rede(e.to_string()))?;
    Ok(b)
}

/// Verifica a imagem anunciada de `app`: assinatura, bloqueio, manifesto e configuração. Não baixa as camadas.
/// `instalada`: a versão em uso (uma imagem assinada mais velha é recusada, salvo `voltar`: a pessoa escolheu voltar).
pub fn verificar(cat: &Catalogo, app: &App, anuncio: &Anuncio, instalada: Option<&str>, voltar: bool) -> Result<Verificada, Falha> {
    if !digest_ok(&anuncio.digest) {
        return Err(Falha::Formato(format!("digest inválido: {}", anuncio.digest)));
    }
    if let Some(b) = app.bloqueio(&anuncio.versao, &anuncio.digest) {
        return Err(Falha::Bloqueada { versao: anuncio.versao.clone(), motivo: b.motivo.clone() });
    }
    let url = format!("{}/{}@{}/signature-1", cat.fonte.assinaturas, app.repositorio, anuncio.digest.replace(':', "="));
    let sig = match texto(&url) {
        Ok(s) => s,
        Err(Falha::Rede(e)) if e.contains("404") => return Err(Falha::Assinatura("a imagem não tem assinatura publicada".into())),
        Err(e) => return Err(e),
    };
    let assinado = conferir_assinatura(cat, &sig)?;
    conferir_texto_assinado(&assinado, &cat.fonte.registro, &app.repositorio, &anuncio.digest)?;
    let f = ficha(cat, &app.repositorio)?;
    let manifesto = do_registro(cat, &app.repositorio, &f, &format!("manifests/{}", anuncio.digest), "application/vnd.oci.image.manifest.v1+json")?;
    if format!("sha256:{}", sha256_hex(&manifesto)) != anuncio.digest {
        return Err(Falha::Assinatura("o manifesto baixado não é o assinado".into()));
    }
    let m: Value = serde_json::from_slice(&manifesto).map_err(|e| Falha::Formato(e.to_string()))?;
    let cd = m["config"]["digest"].as_str().unwrap_or("").to_string();
    let cs = m["config"]["size"].as_u64().unwrap_or(0);
    if !digest_ok(&cd) {
        return Err(Falha::Formato("manifesto sem configuração".into()));
    }
    let camadas: Vec<(String, u64)> = m["layers"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|l| (l["digest"].as_str().unwrap_or("").to_string(), l["size"].as_u64().unwrap_or(0)))
        .collect();
    if camadas.is_empty() || camadas.iter().any(|(d, _)| !digest_ok(d)) {
        return Err(Falha::Formato("manifesto sem camadas válidas".into()));
    }
    let config_bytes = do_registro(cat, &app.repositorio, &f, &format!("blobs/{cd}"), "application/vnd.oci.image.config.v1+json")?;
    if format!("sha256:{}", sha256_hex(&config_bytes)) != cd {
        return Err(Falha::Assinatura("a configuração baixada não é a do manifesto assinado".into()));
    }
    let c: Value = serde_json::from_slice(&config_bytes).map_err(|e| Falha::Formato(e.to_string()))?;
    let l = &c["config"]["Labels"];
    let referencia = l["org.flatpak.ref"].as_str().unwrap_or("").to_string();
    let commit = l["org.flatpak.commit"].as_str().unwrap_or("").to_string();
    // o ref tem de ser o deste app, desta arquitetura
    if referencia != format!("app/{}/{}/{}", app.id, arquitetura().1, cat.fonte.canal) {
        return Err(Falha::Assinatura(format!("a imagem assinada é de outro app ({referencia})")));
    }
    if commit.len() != 64 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Falha::Formato("configuração sem commit do Flatpak".into()));
    }
    // a versão vale a da configuração assinada, não a do índice
    let versao = versao_assinada(&c).ok_or_else(|| Falha::Formato("a configuração assinada não diz a versão".into()))?;
    conferir_versao(app, anuncio, &versao, instalada, voltar)?;
    Ok(Verificada { digest: anuncio.digest.clone(), versao, referencia, commit, config: (cd, cs), camadas, manifesto, config_bytes })
}

/// A versão que a configuração assinada traz (o appdata nas etiquetas da imagem).
pub fn versao_assinada(config: &Value) -> Option<String> {
    versao_do_appdata(config["config"]["Labels"]["org.freedesktop.appstream.appdata"].as_str().unwrap_or(""))
}

/// Bloqueio pela versão assinada, a mesma que o índice anunciou, e nada de voltar atrás sem a pessoa pedir.
pub fn conferir_versao(app: &App, anuncio: &Anuncio, assinada: &str, instalada: Option<&str>, voltar: bool) -> Result<(), Falha> {
    if let Some(b) = app.bloqueio(assinada, &anuncio.digest) {
        return Err(Falha::Bloqueada { versao: assinada.to_string(), motivo: b.motivo.clone() });
    }
    if !anuncio.versao.is_empty() && anuncio.versao != assinada {
        return Err(Falha::Assinatura(format!("o índice anuncia {} e a imagem assinada é {assinada}", anuncio.versao)));
    }
    if let Some(i) = instalada.filter(|i| !i.is_empty()) {
        if !voltar && comparar_versoes(assinada, i) == std::cmp::Ordering::Less {
            return Err(Falha::Assinatura(format!("a imagem assinada ({assinada}) é mais velha que a instalada ({i})")));
        }
    }
    Ok(())
}

/// O andamento de um download (bytes), e o pedido de parar.
#[derive(Clone, Default)]
pub struct Andamento {
    pub feito: Arc<AtomicU64>,
    pub total: Arc<AtomicU64>,
    pub cancelar: Arc<AtomicBool>,
}

/// Baixa as camadas verificadas para um layout OCI em `pasta` (blobs/sha256, index.json, oci-layout), retomando o que
/// já baixou (a rede cai: cada pedaço tem prazo, e o download continua de onde parou). Cada camada é conferida pelo seu
/// sha256 antes de ficar.
pub fn baixar(cat: &Catalogo, app: &App, v: &Verificada, pasta: &Path, a: &Andamento) -> Result<PathBuf, Falha> {
    let blobs = pasta.join("blobs/sha256");
    std::fs::create_dir_all(&blobs).map_err(|e| Falha::Formato(e.to_string()))?;
    a.total.store(v.camadas.iter().map(|c| c.1).sum(), Ordering::Relaxed);
    let mut f = ficha(cat, &app.repositorio)?;
    let mut base = 0u64;
    for (d, tam) in &v.camadas {
        let hex = &d[7..];
        let destino = blobs.join(hex);
        let parcial = blobs.join(format!("{hex}.parcial"));
        let mut tentativas = 0;
        while !destino.is_file() {
            if a.cancelar.load(Ordering::Relaxed) {
                return Err(Falha::Cancelada);
            }
            let ja = std::fs::metadata(&parcial).map(|m| m.len()).unwrap_or(0);
            let mut req = agente()
                .get(&format!("https://{}/v2/{}/blobs/{d}", cat.fonte.registro, app.repositorio))
                .set("Authorization", &format!("Bearer {f}"));
            if ja > 0 {
                req = req.set("Range", &format!("bytes={ja}-"));
            }
            let r = match req.call() {
                Ok(r) => r,
                Err(e) => {
                    tentativas += 1;
                    if tentativas > 8 {
                        return Err(Falha::Rede(e.to_string()));
                    }
                    if matches!(e, ureq::Error::Status(401, _)) {
                        f = ficha(cat, &app.repositorio)?;
                    }
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };
            let continua = r.status() == 206;
            let mut out = std::fs::OpenOptions::new()
                .create(true)
                .append(continua)
                .write(true)
                .truncate(!continua)
                .open(&parcial)
                .map_err(|e| Falha::Formato(e.to_string()))?;
            let mut feito = if continua { ja } else { 0 };
            let mut rd = r.into_reader();
            let mut buf = vec![0u8; 256 << 10];
            let lido = loop {
                if a.cancelar.load(Ordering::Relaxed) {
                    return Err(Falha::Cancelada);
                }
                match rd.read(&mut buf) {
                    Ok(0) => break Ok(()),
                    Ok(n) => {
                        out.write_all(&buf[..n]).map_err(|e| Falha::Formato(e.to_string()))?;
                        feito += n as u64;
                        a.feito.store(base + feito, Ordering::Relaxed);
                    }
                    Err(e) => break Err(e),
                }
            };
            drop(out);
            if lido.is_err() || feito < *tam {
                // (caiu no meio: retoma do que ficou)
                tentativas += 1;
                if tentativas > 30 {
                    return Err(Falha::Rede("o download parou várias vezes".into()));
                }
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
            let bytes = std::fs::read(&parcial).map_err(|e| Falha::Formato(e.to_string()))?;
            if format!("sha256:{}", sha256_hex(&bytes)) != *d {
                let _ = std::fs::remove_file(&parcial);
                return Err(Falha::Assinatura("uma camada baixada não é a do manifesto assinado".into()));
            }
            std::fs::rename(&parcial, &destino).map_err(|e| Falha::Formato(e.to_string()))?;
        }
        base += tam;
        a.feito.store(base, Ordering::Relaxed);
    }
    let mh = &v.digest[7..];
    std::fs::write(blobs.join(mh), &v.manifesto).map_err(|e| Falha::Formato(e.to_string()))?;
    std::fs::write(blobs.join(&v.config.0[7..]), &v.config_bytes).map_err(|e| Falha::Formato(e.to_string()))?;
    std::fs::write(pasta.join("oci-layout"), br#"{"imageLayoutVersion":"1.0.0"}"#).map_err(|e| Falha::Formato(e.to_string()))?;
    let idx = serde_json::json!({"schemaVersion": 2, "manifests": [{"mediaType": "application/vnd.oci.image.manifest.v1+json", "digest": v.digest, "size": v.manifesto.len(),
        // (o Flatpak acha a imagem pelo ref dele neste nome)
        "annotations": {"org.opencontainers.image.ref.name": v.referencia}}]});
    std::fs::write(pasta.join("index.json"), idx.to_string()).map_err(|e| Falha::Formato(e.to_string()))?;
    Ok(pasta.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogo::catalogo;

    #[test]
    fn the_index_gives_each_catalog_app_its_image() {
        let idx = serde_json::json!({"Registry": "https://ghcr.io", "Results": [
            {"Name": "storytold/anicraft", "Images": [
                {"Tags": ["0.2.1-x86_64", "stable-x86_64"], "Digest": "sha256:796ebff6647f18dd4d4838840a39fc91ea5080d79c3d281b59c28c73075d4d9d", "Architecture": "amd64",
                 "Labels": {"org.flatpak.download-size": "19852489", "org.flatpak.installed-size": "46026752", "org.flatpak.metadata": "[Application]\nname=ai.storyteller.anicraft\nruntime=org.freedesktop.Platform/x86_64/26.08\n",
                  "org.freedesktop.appstream.appdata": "<component><releases><release version=\"0.2.1\" date=\"2026-11-20\"/></releases></component>"}},
                {"Tags": ["0.2.1-aarch64", "stable-aarch64"], "Digest": "sha256:aa", "Architecture": "arm64", "Labels": {}}]},
            {"Name": "storytold/some-other", "Images": []}]});
        let v = ler_indice(catalogo(), idx.to_string().as_bytes()).unwrap();
        if arquitetura().0 != "amd64" {
            return;
        }
        assert_eq!(v.len(), 1);
        let (id, a) = &v[0];
        assert_eq!(id, "ai.storyteller.anicraft");
        assert_eq!(a.versao, "0.2.1");
        assert_eq!(a.tamanho_download, 19852489);
        assert_eq!(a.runtime, "org.freedesktop.Platform/x86_64/26.08");
    }

    #[test]
    fn the_signed_text_must_name_this_image_and_repository() {
        let d = "sha256:796ebff6647f18dd4d4838840a39fc91ea5080d79c3d281b59c28c73075d4d9d";
        let t = |tipo: &str, dg: &str, r: &str| {
            serde_json::json!({"critical": {"type": tipo, "image": {"docker-manifest-digest": dg}, "identity": {"docker-reference": r}}}).to_string()
        };
        assert!(conferir_texto_assinado(
            t("atomic container signature", d, "ghcr.io/storytold/anicraft").as_bytes(),
            "ghcr.io",
            "storytold/anicraft",
            d
        )
        .is_ok());
        assert!(matches!(
            conferir_texto_assinado(t("atomic container signature", d, "ghcr.io/storytold/filmcraft").as_bytes(), "ghcr.io", "storytold/anicraft", d),
            Err(Falha::Assinatura(_))
        ));
        assert!(matches!(
            conferir_texto_assinado(
                t("atomic container signature", "sha256:00", "ghcr.io/storytold/anicraft").as_bytes(),
                "ghcr.io",
                "storytold/anicraft",
                d
            ),
            Err(Falha::Assinatura(_))
        ));
        assert!(matches!(
            conferir_texto_assinado(t("other", d, "ghcr.io/storytold/anicraft").as_bytes(), "ghcr.io", "storytold/anicraft", d),
            Err(Falha::Assinatura(_))
        ));
    }

    /// The project's real signature of FilmCraft 0.20.1 (from getartcraft.com/oci-signatures, 2026-11-24).
    const REAL: &[u8] = include_bytes!("testes/filmcraft-0.20.1.signature-1");

    #[test]
    fn the_projects_real_signature_verifies_with_the_pinned_key() {
        if std::process::Command::new(gpgv()).arg("--version").output().is_err() {
            eprintln!("gpgv missing: skipped");
            return;
        }
        let texto = conferir_assinatura(catalogo(), REAL).unwrap();
        let d = "sha256:9deced2d7c08842bb9cfd6e8164228e9ba85952c8239eb89396fd507a292e04e";
        conferir_texto_assinado(&texto, "ghcr.io", "storytold/filmcraft", d).unwrap();
        // the same signature does not cover another app
        assert!(conferir_texto_assinado(&texto, "ghcr.io", "storytold/anicraft", d).is_err());
        // one byte changed: refused
        let mut mexida = REAL.to_vec();
        let n = mexida.len() / 2;
        mexida[n] ^= 0x20;
        assert!(matches!(conferir_assinatura(catalogo(), &mexida), Err(Falha::Assinatura(_))));
    }

    #[test]
    fn a_signature_by_a_key_other_than_the_pinned_one_is_refused() {
        if std::process::Command::new(gpgv()).arg("--version").output().is_err() {
            return;
        }
        // the same key file, but the catalog pins another fingerprint
        let mut c = catalogo().clone();
        c.fonte.impressao_digital = "0000000000000000000000000000000000000000".into();
        assert!(matches!(conferir_assinatura(&c, REAL), Err(Falha::Assinatura(_))));
    }

    #[test]
    fn the_signed_version_decides_blocks_mismatches_and_downgrades() {
        let mut a = catalogo().app("effectcraft").unwrap().clone();
        a.bloqueadas.push(crate::catalogo::Bloqueio { versao: "0.25.1".into(), digest: String::new(), motivo: "falha ao exportar".into() });
        let an = |v: &str| Anuncio { digest: format!("sha256:{}", "b".repeat(64)), versao: v.into(), ..Default::default() };
        // the index lies (announces 0.25.2) but the signed config says 0.25.1: blocked by the signed version
        assert!(matches!(conferir_versao(&a, &an("0.25.2"), "0.25.1", None, false), Err(Falha::Bloqueada { .. })));
        // announced and signed differ: refused
        assert!(matches!(conferir_versao(&a, &an("0.26.0"), "0.25.9", None, false), Err(Falha::Assinatura(_))));
        // older than installed: refused unless the person chose to go back
        assert!(matches!(conferir_versao(&a, &an("0.24.0"), "0.24.0", Some("0.25.0"), false), Err(Falha::Assinatura(_))));
        assert!(conferir_versao(&a, &an("0.24.0"), "0.24.0", Some("0.25.0"), true).is_ok());
        assert!(conferir_versao(&a, &an("0.26.0"), "0.26.0", Some("0.25.0"), false).is_ok());
        let cfg = serde_json::json!({"config": {"Labels": {"org.freedesktop.appstream.appdata": "<component><releases><release version=\"0.25.1\" date=\"x\"/></releases></component>"}}});
        assert_eq!(versao_assinada(&cfg).as_deref(), Some("0.25.1"));
        assert_eq!(comparar_versoes("0.10.0", "0.9.9"), std::cmp::Ordering::Greater);
    }

    #[test]
    fn a_garbage_or_foreign_signature_is_refused() {
        // (gpgv may be missing on a build machine: then it is refused too)
        assert!(matches!(conferir_assinatura(catalogo(), b"not a signature"), Err(Falha::Assinatura(_))));
    }

    #[test]
    fn a_blocked_version_is_refused_before_anything_is_fetched() {
        let mut a = catalogo().app("effectcraft").unwrap().clone();
        a.bloqueadas.push(crate::catalogo::Bloqueio { versao: "0.25.1".into(), digest: String::new(), motivo: "falha conhecida ao exportar".into() });
        let an = Anuncio { digest: format!("sha256:{}", "a".repeat(64)), versao: "0.25.1".into(), ..Default::default() };
        assert_eq!(
            verificar(catalogo(), &a, &an, None, false),
            Err(Falha::Bloqueada { versao: "0.25.1".into(), motivo: "falha conhecida ao exportar".into() })
        );
    }
}
