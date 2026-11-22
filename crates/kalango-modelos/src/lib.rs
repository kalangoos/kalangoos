//! Where the AI models come from. The published image ships no model: the installer records a source (the public
//! catalog, a private server, a folder or a USB drive, or "later") and the installed system downloads what was
//! chosen, checking every file's size and SHA-256 before it is kept.
//!
//! - [`catalog`]: the model catalog (`kalango/loja/modelos.json`), with each file's SHA-256;
//! - [`config`]: `/etc/kalango/modelos.toml`, the choice (never the access token);
//! - [`urlpolicy`]: which server addresses are accepted and where a request may be redirected;
//! - [`sums`]: `SHA256SUMS` files;
//! - [`net`]: HTTP with the redirect and token rules;
//! - [`probe`]: "test the connection": what a server or a folder has, and whether its sums match the catalog;
//! - [`download`]: the download engine (resume, streaming SHA-256, verify before keeping);
//! - [`queue`]: the downloads running in the background, for the Central de Apps;
//! - [`vault`]: the access token (system vault, with a first-boot hand-off from the installer).

pub mod catalog;
pub mod config;
pub mod download;
pub mod net;
pub mod probe;
pub mod queue;
pub mod sums;
pub mod urlpolicy;
pub mod vault;

/// A secret that never prints itself (debug output, logs and test failures show it as stars).
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Secret {
        Secret(s.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}
