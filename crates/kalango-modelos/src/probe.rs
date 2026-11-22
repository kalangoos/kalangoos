//! "Test the connection": what a private server or a folder has, and whether each model's files match the catalog,
//! without downloading anything (the server's `SHA256SUMS` and HEAD requests give sums and sizes).

use crate::catalog::{ModelEntry, ModelFile};
use crate::download::{hash_file, Control};
use crate::net::{Client, NetError};
use crate::sums::parse_sums;
use crate::urlpolicy::ServerUrl;
use std::io::Read;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelState {
    /// there, with the size and the sum the catalog says
    Verified,
    /// there, but no sum to check (no SHA256SUMS on the server)
    Unverified,
    /// the sum differs from the catalog's: refused
    SumMismatch,
    /// there with another size than the catalog's
    WrongSize,
    /// not on the server / not in the folder
    Absent,
    /// the catalog has no SHA-256 for it: it cannot come from this source
    NoChecksum,
}

impl ModelState {
    /// Can it be ticked? (a refused model cannot)
    pub fn usable(self) -> bool {
        matches!(self, ModelState::Verified | ModelState::Unverified)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    pub models: Vec<(String, ModelState)>,
    /// the server published a SHA256SUMS
    pub sums_found: bool,
}

impl Probe {
    pub fn state(&self, id: &str) -> Option<ModelState> {
        self.models.iter().find(|(i, _)| i == id).map(|(_, s)| *s)
    }
    pub fn found(&self) -> usize {
        self.models.iter().filter(|(_, s)| s.usable()).count()
    }
    pub fn all_verified(&self) -> bool {
        let f = self.found();
        f > 0 && self.models.iter().all(|(_, s)| !s.usable() || *s == ModelState::Verified)
    }
}

fn combine(files: impl Iterator<Item = ModelState>) -> ModelState {
    let v: Vec<_> = files.collect();
    for bad in [ModelState::NoChecksum, ModelState::Absent, ModelState::SumMismatch, ModelState::WrongSize] {
        if v.contains(&bad) {
            return bad;
        }
    }
    if v.iter().all(|s| *s == ModelState::Verified) { ModelState::Verified } else { ModelState::Unverified }
}

fn content_length(r: &ureq::Response) -> Option<u64> {
    r.header("Content-Length").and_then(|v| v.parse().ok())
}

/// A private server: GET `<base>/SHA256SUMS`, then HEAD each file. Only a failure of the connection itself or of the
/// access (401/403) is an error; a missing file is "absent".
pub fn probe_server(client: &Client, server: &ServerUrl, catalog: &[ModelEntry]) -> Result<Probe, NetError> {
    let sums = match client.request("GET", &server.file_url("SHA256SUMS"), None) {
        Ok(r) => {
            let mut t = String::new();
            r.into_reader().take(1 << 20).read_to_string(&mut t).map_err(|e| NetError::Unreachable(e.to_string()))?;
            Some(parse_sums(&t))
        }
        Err(NetError::NotFound) => None,
        Err(e) => return Err(e),
    };
    let mut models = Vec::new();
    for m in catalog {
        if !m.verifiable() {
            models.push((m.id.clone(), ModelState::NoChecksum));
            continue;
        }
        let mut states = Vec::new();
        for f in &m.arquivos {
            states.push(match client.request("HEAD", &server.file_url(&f.nome), None) {
                Ok(r) => match content_length(&r) {
                    Some(n) if n != f.bytes => ModelState::WrongSize,
                    _ => match sums.as_ref().and_then(|s| s.get(&f.nome)) {
                        Some(s) if *s == f.sha256 => ModelState::Verified,
                        Some(_) => ModelState::SumMismatch,
                        None => ModelState::Unverified,
                    },
                },
                Err(NetError::NotFound) => ModelState::Absent,
                Err(e) => return Err(e),
            });
        }
        models.push((m.id.clone(), combine(states.into_iter())));
    }
    Ok(Probe { models, sums_found: sums.is_some() })
}

fn check_file(dir: &Path, f: &ModelFile, ctl: &Control) -> ModelState {
    let p = dir.join(&f.nome);
    let Ok(md) = std::fs::metadata(&p) else { return ModelState::Absent };
    if !md.is_file() {
        return ModelState::Absent;
    }
    if md.len() != f.bytes {
        return ModelState::WrongSize;
    }
    match hash_file(&p, ctl) {
        Ok(h) if h == f.sha256 => ModelState::Verified,
        Ok(_) => ModelState::SumMismatch,
        Err(_) => ModelState::Absent,
    }
}

/// A folder or USB drive: the files by their names, size and the SHA-256 computed on the spot.
pub fn probe_folder(dir: &Path, catalog: &[ModelEntry], ctl: &Control) -> Probe {
    let models = catalog.iter().map(|m| {
        let s = if !m.verifiable() { ModelState::NoChecksum } else { combine(m.arquivos.iter().map(|f| check_file(dir, f, ctl))) };
        (m.id.clone(), s)
    }).collect();
    Probe { models, sums_found: false }
}
