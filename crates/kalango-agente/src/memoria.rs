//! What the agent remembers about the person (the agents center's "Memória"): a readable list, never a black box.
//! Each memory says where it came from ("voce": the person said it in a conversation; "observado": learned from what
//! the person does, by the shell; "marcado": the person marked it; "arquivo": from a file, with its folder) and can be
//! corrected or deleted. The agent sees the memories in its prompt as data, never as instructions.
//!
//! Stored only on this computer, in `~/.local/share/kalango/memoria.json` (0600, a folder kalangod protects from every
//! agent); encrypted (ChaCha20-Poly1305, `memoria.bin`) when the system keyring (Secret Service) is there to keep the
//! key. `KALANGO_MEMORIA_CHAVEIRO=nao` keeps tests away from any keyring.
//!
//! Anonymous mode lasts for the session (a marker in XDG_RUNTIME_DIR): nothing new is remembered while it is on.

use serde_json::{json, Value};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

pub const CATEGORIAS: [&str; 4] = ["preferencias", "rotinas", "projetos", "pessoas"];

#[derive(Clone, Debug, PartialEq)]
pub struct Lembranca {
    pub id: u64,
    pub texto: String,
    pub categoria: String,
    pub origem: String,
    pub fonte: Option<String>,
    pub criado: i64,
    pub atualizado: i64,
    /// the workspace it belongs to, when memory is kept apart by space
    pub espaco: Option<String>,
    /// observed facts: the same key replaces the old version
    pub chave: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ajustes {
    pub ligada: bool,
    pub aprender: bool,
    pub nuvem: bool,
    pub por_espaco: bool,
}

impl Default for Ajustes {
    fn default() -> Ajustes {
        Ajustes { ligada: true, aprender: true, nuvem: false, por_espaco: false }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Memoria {
    pub proximo: u64,
    pub lembrancas: Vec<Lembranca>,
    pub ajustes: Ajustes,
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl Lembranca {
    fn from_json(v: &Value) -> Option<Lembranca> {
        Some(Lembranca {
            id: v["id"].as_u64()?,
            texto: v["texto"].as_str()?.to_string(),
            categoria: v["categoria"].as_str().filter(|c| CATEGORIAS.contains(c)).unwrap_or("preferencias").to_string(),
            origem: v["origem"].as_str().unwrap_or("voce").to_string(),
            fonte: v["fonte"].as_str().map(str::to_string),
            criado: v["criado"].as_i64().unwrap_or(0),
            atualizado: v["atualizado"].as_i64().unwrap_or(0),
            espaco: v["espaco"].as_str().map(str::to_string),
            chave: v["chave"].as_str().map(str::to_string),
        })
    }

    pub fn to_json(&self) -> Value {
        json!({"id": self.id, "texto": self.texto, "categoria": self.categoria, "origem": self.origem, "fonte": self.fonte,
            "criado": self.criado, "atualizado": self.atualizado, "espaco": self.espaco, "chave": self.chave})
    }
}

impl Memoria {
    pub fn parse(v: &Value) -> Memoria {
        let d = Ajustes::default();
        let a = &v["ajustes"];
        Memoria {
            proximo: v["proximo"].as_u64().unwrap_or(1),
            lembrancas: v["lembrancas"].as_array().into_iter().flatten().filter_map(Lembranca::from_json).collect(),
            ajustes: Ajustes {
                ligada: a["ligada"].as_bool().unwrap_or(d.ligada), aprender: a["aprender"].as_bool().unwrap_or(d.aprender),
                nuvem: a["nuvem"].as_bool().unwrap_or(d.nuvem), por_espaco: a["por_espaco"].as_bool().unwrap_or(d.por_espaco),
            },
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "proximo": self.proximo, "lembrancas": self.lembrancas.iter().map(Lembranca::to_json).collect::<Vec<_>>(),
            "ajustes": {"ligada": self.ajustes.ligada, "aprender": self.ajustes.aprender, "nuvem": self.ajustes.nuvem, "por_espaco": self.ajustes.por_espaco},
        })
    }

    /// Adds what the person said (the same text again only refreshes it). Returns its id.
    pub fn lembrar(&mut self, texto: &str, categoria: &str, origem: &str, espaco: Option<String>) -> u64 {
        let texto = texto.trim().trim_end_matches('.').to_string();
        let agora = now();
        if let Some(l) = self.lembrancas.iter_mut().find(|l| l.texto.eq_ignore_ascii_case(&texto)) {
            l.atualizado = agora;
            return l.id;
        }
        let id = self.proximo.max(1);
        self.proximo = id + 1;
        let categoria = if CATEGORIAS.contains(&categoria) { categoria } else { "preferencias" };
        self.lembrancas.push(Lembranca { id, texto, categoria: categoria.into(), origem: origem.into(), fonte: None, criado: agora, atualizado: agora, espaco, chave: None });
        id
    }

    /// Forgets the memories that contain every word of `o_que` (accents and case aside). Returns what was forgotten.
    pub fn esquecer(&mut self, o_que: &str) -> Vec<String> {
        let palavras: Vec<String> = simples(o_que).split_whitespace().filter(|w| w.len() > 2).map(str::to_string).collect();
        if palavras.is_empty() {
            return vec![];
        }
        let mut foram = Vec::new();
        self.lembrancas.retain(|l| {
            let t = simples(&l.texto);
            let sai = palavras.iter().all(|w| t.contains(w.as_str()));
            if sai {
                foram.push(l.texto.clone());
            }
            !sai
        });
        foram
    }

    /// The facts the shell observed now: each key replaces its old version; observed memories whose key is gone (the
    /// habit faded) go too.
    pub fn observar(&mut self, fatos: &[(String, String, String)]) {
        let agora = now();
        self.lembrancas.retain(|l| l.origem != "observado" || l.chave.as_ref().is_some_and(|k| fatos.iter().any(|(c, _, _)| c == k)));
        for (chave, texto, categoria) in fatos {
            match self.lembrancas.iter_mut().find(|l| l.chave.as_deref() == Some(chave.as_str())) {
                Some(l) if l.texto == *texto => {}
                Some(l) => {
                    l.texto = texto.clone();
                    l.atualizado = agora;
                }
                None => {
                    let id = self.proximo.max(1);
                    self.proximo = id + 1;
                    self.lembrancas.push(Lembranca {
                        id, texto: texto.clone(), categoria: if CATEGORIAS.contains(&categoria.as_str()) { categoria.clone() } else { "rotinas".into() },
                        origem: "observado".into(), fonte: None, criado: agora, atualizado: agora, espaco: None, chave: Some(chave.clone()),
                    });
                }
            }
        }
    }

    /// The memories the agent sees now (all, or those of this space and the ones of no space).
    pub fn para_o_agente(&self, espaco: Option<&str>) -> Vec<&Lembranca> {
        if !self.ajustes.ligada {
            return vec![];
        }
        self.lembrancas.iter().filter(|l| !self.ajustes.por_espaco || l.espaco.is_none() || l.espaco.as_deref() == espaco).collect()
    }
}

/// Lowercase without accents (for matching what to forget).
fn simples(s: &str) -> String {
    s.to_lowercase().chars().map(|c| match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
        'é' | 'ê' | 'è' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        c => c,
    }).collect()
}

// ------------------------------------------------------------------ where it is kept

fn pasta() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")).join("kalango")
}

fn arquivo_aberto() -> PathBuf {
    pasta().join("memoria.json")
}

fn arquivo_cifrado() -> PathBuf {
    pasta().join("memoria.bin")
}

fn anonimo_marca() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("kalango/memoria-anonima")
}

pub fn anonimo() -> bool {
    anonimo_marca().exists()
}

pub fn set_anonimo(on: bool) -> Result<(), String> {
    let m = anonimo_marca();
    if on {
        if let Some(d) = m.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(&m, b"").map_err(|e| e.to_string())
    } else {
        match std::fs::remove_file(&m) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }
}

/// secret-tool with a time limit (a session without a keyring service would otherwise wait two minutes).
fn chaveiro(args: &[&str], entrada: Option<&str>) -> Option<String> {
    if std::env::var("KALANGO_MEMORIA_CHAVEIRO").as_deref() == Ok("nao") || std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        return None;
    }
    let mut c = Command::new("secret-tool").args(args).stdin(if entrada.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    if let (Some(e), Some(mut i)) = (entrada, c.stdin.take()) {
        let _ = i.write_all(e.as_bytes());
    }
    let fim = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        match c.try_wait() {
            Ok(Some(s)) if s.success() => {
                let mut out = String::new();
                use std::io::Read;
                c.stdout.take()?.read_to_string(&mut out).ok()?;
                return Some(out.trim().to_string());
            }
            Ok(Some(_)) => return None,
            Ok(None) if std::time::Instant::now() < fim => std::thread::sleep(Duration::from_millis(30)),
            _ => {
                let _ = c.kill();
                let _ = c.wait();
                return None;
            }
        }
    }
}

const ATRIBUTOS: [&str; 4] = ["app", "kalango", "tipo", "memoria"];

fn chave_existente() -> Option<[u8; 32]> {
    use base64::Engine;
    let s = chaveiro(&[&["lookup"][..], &ATRIBUTOS[..]].concat(), None)?;
    base64::engine::general_purpose::STANDARD.decode(s).ok()?.try_into().ok()
}

fn chave_nova() -> Option<[u8; 32]> {
    use base64::Engine;
    use ring::rand::SecureRandom;
    let mut k = [0u8; 32];
    ring::rand::SystemRandom::new().fill(&mut k).ok()?;
    let b = base64::engine::general_purpose::STANDARD.encode(k);
    chaveiro(&[&["store", "--label=KalangoOS · memória do agente"][..], &ATRIBUTOS[..]].concat(), Some(&b))?;
    // stored only if it reads back
    (chave_existente()? == k).then_some(k)
}

fn cifrar(k: &[u8; 32], dados: &[u8]) -> Option<Vec<u8>> {
    use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};
    use ring::rand::SecureRandom;
    let chave = LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, k).ok()?);
    let mut n = [0u8; 12];
    ring::rand::SystemRandom::new().fill(&mut n).ok()?;
    let mut buf = dados.to_vec();
    chave.seal_in_place_append_tag(Nonce::assume_unique_for_key(n), Aad::empty(), &mut buf).ok()?;
    Some([&n[..], &buf].concat())
}

fn decifrar(k: &[u8; 32], dados: &[u8]) -> Option<Vec<u8>> {
    use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};
    if dados.len() < 12 + 16 {
        return None;
    }
    let chave = LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, k).ok()?);
    let n: [u8; 12] = dados[..12].try_into().ok()?;
    let mut buf = dados[12..].to_vec();
    let claro = chave.open_in_place(Nonce::assume_unique_for_key(n), Aad::empty(), &mut buf).ok()?;
    Some(claro.to_vec())
}

/// Whether the memory is kept encrypted (the page says so only when it is).
pub fn cifrada() -> bool {
    arquivo_cifrado().exists()
}

pub fn tamanho() -> u64 {
    [arquivo_cifrado(), arquivo_aberto()].iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum()
}

pub fn carregar() -> Result<Memoria, String> {
    if let Ok(d) = std::fs::read(arquivo_cifrado()) {
        let k = chave_existente().ok_or_else(|| kalango_i18n::tr("the memory is encrypted and the system keyring is locked or missing").to_string())?;
        let claro = decifrar(&k, &d).ok_or_else(|| kalango_i18n::tr("the memory could not be decrypted").to_string())?;
        return serde_json::from_slice(&claro).map(|v| Memoria::parse(&v)).map_err(|e| e.to_string());
    }
    match std::fs::read(arquivo_aberto()) {
        Ok(d) => serde_json::from_slice(&d).map(|v| Memoria::parse(&v)).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Memoria { proximo: 1, ..Default::default() }),
        Err(e) => Err(e.to_string()),
    }
}

fn escrever(p: &std::path::Path, dados: &[u8]) -> Result<(), String> {
    let tmp = p.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).map_err(|e| e.to_string())?;
    f.write_all(dados).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, p).map_err(|e| e.to_string())
}

pub fn salvar(m: &Memoria) -> Result<(), String> {
    std::fs::create_dir_all(pasta()).map_err(|e| e.to_string())?;
    let claro = m.to_json().to_string();
    // encrypted when it already is, or when this is the first time and a keyring takes the key
    let primeira = !arquivo_cifrado().exists() && !arquivo_aberto().exists();
    let chave = if arquivo_cifrado().exists() { chave_existente() } else if primeira { chave_existente().or_else(chave_nova) } else { None };
    match chave.and_then(|k| cifrar(&k, claro.as_bytes())) {
        Some(c) => {
            escrever(&arquivo_cifrado(), &c)?;
            let _ = std::fs::remove_file(arquivo_aberto());
            Ok(())
        }
        None if arquivo_cifrado().exists() => Err(kalango_i18n::tr("the memory is encrypted and the system keyring is locked or missing").into()),
        None => escrever(&arquivo_aberto(), claro.as_bytes()),
    }
}

/// The name of the focused workspace (niri), for memory kept apart by space.
pub fn espaco_atual() -> Option<String> {
    use std::io::{BufRead, BufReader};
    let s = std::os::unix::net::UnixStream::connect(std::env::var_os("NIRI_SOCKET")?).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    let mut w = s.try_clone().ok()?;
    writeln!(w, "\"Workspaces\"").ok()?;
    let mut l = String::new();
    BufReader::new(s).read_line(&mut l).ok()?;
    let v: Value = serde_json::from_str(&l).ok()?;
    v["Ok"]["Workspaces"].as_array()?.iter().find(|w| w["is_focused"] == true)
        .map(|w| w["name"].as_str().map(str::to_string).unwrap_or_else(|| w["idx"].to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_forgets_and_observes() {
        let mut m = Memoria { proximo: 1, ..Default::default() };
        let a = m.lembrar("Prefere tema escuro.", "preferencias", "voce", None);
        assert_eq!(m.lembrar("prefere tema escuro", "preferencias", "voce", None), a, "the same text only refreshes it");
        m.lembrar("Responder em português, direto ao ponto", "preferencias", "voce", Some("Trabalho".into()));
        assert_eq!(m.esquecer("portugues"), vec!["Responder em português, direto ao ponto".to_string()]);
        assert_eq!(m.lembrancas.len(), 1);
        // observed facts replace their older version and go when the habit is gone
        m.observar(&[("app:term".into(), "Costuma abrir o Terminal entre 20h e 2h".into(), "rotinas".into())]);
        m.observar(&[("app:term".into(), "Costuma abrir o Terminal entre 21h e 1h".into(), "rotinas".into())]);
        assert_eq!(m.lembrancas.iter().filter(|l| l.origem == "observado").count(), 1);
        assert!(m.lembrancas.iter().any(|l| l.texto.contains("21h")));
        m.observar(&[]);
        assert!(m.lembrancas.iter().all(|l| l.origem != "observado"));
        assert_eq!(Memoria::parse(&m.to_json()), m);
    }

    #[test]
    fn spaces_and_switch() {
        let mut m = Memoria { proximo: 1, ..Default::default() };
        m.lembrar("geral", "preferencias", "voce", None);
        m.lembrar("só do trabalho", "projetos", "voce", Some("Trabalho".into()));
        assert_eq!(m.para_o_agente(Some("Pessoal")).len(), 2, "not apart: everything");
        m.ajustes.por_espaco = true;
        assert_eq!(m.para_o_agente(Some("Pessoal")).len(), 1);
        assert_eq!(m.para_o_agente(Some("Trabalho")).len(), 2);
        m.ajustes.ligada = false;
        assert!(m.para_o_agente(Some("Trabalho")).is_empty());
    }

    #[test]
    fn encryption_round_trip() {
        let k = [7u8; 32];
        let c = cifrar(&k, b"segredo").unwrap();
        assert_ne!(&c[12..], b"segredo");
        assert_eq!(decifrar(&k, &c).unwrap(), b"segredo");
        assert!(decifrar(&[8u8; 32], &c).is_none(), "another key does not open it");
    }
}
