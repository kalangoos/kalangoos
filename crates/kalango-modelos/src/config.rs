//! `/etc/kalango/modelos.toml`: the models' source the installer recorded (and Settings may change). It holds no
//! secret: the access token lives in the system vault ([`crate::vault`]).
//!
//! ```toml
//! fonte = "servidor"            # "padrao" | "servidor" | "pasta" | "depois"
//! servidor = "https://modelos.example.org/kalango"
//! pasta = "/media/pendrive/modelos"
//! escolhidos = ["lfm3-1.7b", "lfm3-audio-2b"]
//! rede_local_confirmada = false  # only for an http:// server on a private network
//! ```

use crate::catalog::DEFAULT_CHOSEN;
use crate::urlpolicy::{parse_server, ServerUrl, UrlError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SYSTEM_PATH: &str = "/etc/kalango/modelos.toml";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    #[default]
    Padrao,
    Servidor,
    Pasta,
    Depois,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub fonte: SourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub servidor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pasta: Option<String>,
    pub escolhidos: Vec<String>,
    pub rede_local_confirmada: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { fonte: SourceKind::Padrao, servidor: None, pasta: None, escolhidos: DEFAULT_CHOSEN.iter().map(|s| s.to_string()).collect(), rede_local_confirmada: false }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    Parse(String),
    Server(UrlError),
    MissingServer,
    MissingFolder,
    RelativeFolder,
}

impl Config {
    /// The file's path: `KALANGO_MODELOS_CONF` (tests, the session) or the system's.
    pub fn path() -> PathBuf {
        std::env::var_os("KALANGO_MODELOS_CONF").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(SYSTEM_PATH))
    }

    pub fn parse(text: &str) -> Result<Config, ConfigError> {
        let c: Config = toml::from_str(text).map_err(|e| ConfigError::Parse(e.message().to_string()))?;
        c.validate()?;
        Ok(c)
    }

    /// The server's address checked by the URL policy.
    pub fn server(&self) -> Result<ServerUrl, ConfigError> {
        let s = self.servidor.as_deref().filter(|s| !s.trim().is_empty()).ok_or(ConfigError::MissingServer)?;
        parse_server(s, self.rede_local_confirmada).map_err(ConfigError::Server)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        match self.fonte {
            SourceKind::Servidor => self.server().map(|_| ()),
            SourceKind::Pasta => match self.pasta.as_deref().filter(|p| !p.trim().is_empty()) {
                None => Err(ConfigError::MissingFolder),
                Some(p) if !Path::new(p).is_absolute() => Err(ConfigError::RelativeFolder),
                Some(_) => Ok(()),
            },
            _ => Ok(()),
        }
    }

    pub fn to_toml(&self) -> String {
        format!("# KalangoOS: where the AI models come from (written by the installer; Settings > Models changes it).\n# The access token is not here: it is in the system vault.\n{}", toml::to_string(self).unwrap_or_default())
    }

    /// Reads the file. A missing file means "nothing was recorded" (the default source); a file that cannot be read
    /// or is not valid is an error, never a silent fall-back to the public source.
    pub fn try_load(path: &Path) -> Result<Config, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(t) => Config::parse(&t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(ConfigError::Parse(e.to_string())),
        }
    }

    pub fn load_system() -> Result<Config, ConfigError> {
        Config::try_load(&Config::path())
    }

    /// The person's own choice (Settings), kept in their config folder; it wins over the system's.
    pub fn user_path() -> PathBuf {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
            .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")).join("kalango/modelos.toml")
    }

    /// What is in force: the person's file when there is one, else the system's (the installer's), else the default.
    /// A person's file that is broken is an error: it does not fall back to the system's choice.
    pub fn load_effective() -> Result<Config, ConfigError> {
        let u = Config::user_path();
        if u.exists() { Config::try_load(&u) } else { Config::load_system() }
    }

    /// Like [`Config::source`], with a token the person just typed (not yet in the vault).
    pub fn source_with(&self, typed: Option<crate::Secret>) -> crate::download::Source {
        match (self.source(), typed) {
            (crate::download::Source::Server { url, .. }, Some(t)) => crate::download::Source::Server { url, token: Some(t) },
            (s, _) => s,
        }
    }

    /// Where downloads come from under this configuration. A private server gets the token only when the token was
    /// kept for that very server (see [`crate::vault`]); an invalid configuration downloads nothing.
    pub fn source(&self) -> crate::download::Source {
        use crate::download::Source;
        match self.fonte {
            SourceKind::Servidor => match self.server() {
                Ok(url) => {
                    let token = crate::vault::read_for(&url.origin());
                    Source::Server { url, token }
                }
                Err(_) => Source::Disabled,
            },
            SourceKind::Pasta => match self.pasta.as_deref().filter(|p| p.starts_with('/')) {
                Some(p) => Source::Folder(p.into()),
                None => Source::Disabled,
            },
            SourceKind::Padrao | SourceKind::Depois => Source::Default,
        }
    }

    /// Writes it atomically (world-readable: it has no secret).
    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.validate().map_err(|e| format!("{e:?}"))?;
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, self.to_toml()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_the_public_source_with_text_voice_and_vision() {
        let c = Config::default();
        assert_eq!(c.fonte, SourceKind::Padrao);
        assert_eq!(c.escolhidos, ["lfm3-1.7b", "lfm3-audio-2b", "lfm3-vl-2b"]);
        assert_eq!(Config::parse("").unwrap(), c);
    }

    #[test]
    fn round_trips_a_private_server() {
        let c = Config { fonte: SourceKind::Servidor, servidor: Some("https://modelos.example.org/k".into()), escolhidos: vec!["lfm3-1.7b".into()], ..Default::default() };
        let t = c.to_toml();
        assert!(t.contains("fonte = \"servidor\""));
        assert_eq!(Config::parse(&t).unwrap(), c);
    }

    #[test]
    fn a_server_needs_a_valid_address() {
        assert_eq!(Config::parse("fonte = \"servidor\"").unwrap_err(), ConfigError::MissingServer);
        assert_eq!(Config::parse("fonte = \"servidor\"\nservidor = \"http://modelos.example.org\"").unwrap_err(), ConfigError::Server(UrlError::PlainHttp));
        assert!(Config::parse("fonte = \"servidor\"\nservidor = \"http://192.168.0.9\"\nrede_local_confirmada = true").is_ok());
    }

    #[test]
    fn a_folder_must_be_absolute() {
        assert_eq!(Config::parse("fonte = \"pasta\"").unwrap_err(), ConfigError::MissingFolder);
        assert_eq!(Config::parse("fonte = \"pasta\"\npasta = \"modelos\"").unwrap_err(), ConfigError::RelativeFolder);
        assert!(Config::parse("fonte = \"pasta\"\npasta = \"/media/x\"").is_ok());
    }

    #[test]
    fn unknown_source_is_refused_and_missing_file_is_the_default() {
        assert!(Config::parse("fonte = \"nuvem\"").is_err());
        let d = tempfile::tempdir().unwrap();
        assert_eq!(Config::try_load(&d.path().join("nao-existe.toml")), Ok(Config::default()));
        // a file that is there but broken is an error, not the public source
        let bad = d.path().join("quebrado.toml");
        std::fs::write(&bad, "fonte = [").unwrap();
        assert!(matches!(Config::try_load(&bad), Err(ConfigError::Parse(_))));
        std::fs::write(&bad, "fonte = \"servidor\"\nservidor = \"http://modelos.example.org\"").unwrap();
        assert_eq!(Config::try_load(&bad), Err(ConfigError::Server(UrlError::PlainHttp)));
        assert!(Config::parse("fonte = \"nuvem\"").is_err());
    }

    #[test]
    fn the_persons_file_wins_over_the_systems() {
        let d = tempfile::tempdir().unwrap();
        let sys = d.path().join("sys.toml");
        Config { fonte: SourceKind::Depois, escolhidos: vec![], ..Default::default() }.save(&sys).unwrap();
        let user = d.path().join("cfg/kalango/modelos.toml");
        // (process-wide variables: this is the only test of the crate's unit tests that sets them)
        std::env::set_var("KALANGO_MODELOS_CONF", &sys);
        std::env::set_var("XDG_CONFIG_HOME", d.path().join("cfg"));
        assert_eq!(Config::user_path(), user);
        assert_eq!(Config::load_effective().unwrap().fonte, SourceKind::Depois);
        Config { fonte: SourceKind::Pasta, pasta: Some("/media/x".into()), ..Default::default() }.save(&user).unwrap();
        assert_eq!(Config::load_effective().unwrap().fonte, SourceKind::Pasta);
        // a broken file of the person does not fall back to the system's
        std::fs::write(&user, "fonte = [").unwrap();
        assert!(Config::load_effective().is_err());
        std::env::remove_var("KALANGO_MODELOS_CONF");
        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[test]
    fn save_then_load_and_no_token_field_exists() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("etc/kalango/modelos.toml");
        let c = Config { fonte: SourceKind::Depois, escolhidos: vec![], ..Default::default() };
        c.save(&p).unwrap();
        assert_eq!(Config::try_load(&p), Ok(c));
        assert!(!std::fs::read_to_string(&p).unwrap().to_lowercase().contains("token ="));
    }
}
