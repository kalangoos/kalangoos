//! Níveis de poder, configuração e decisão de acesso.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Os quatro níveis de poder do kalangod (docs/ARQUITETURA.md, seção 3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Ler o estado do sistema.
    Observar = 0,
    /// Agir como o usuário, nas pastas permitidas.
    Usuario = 1,
    /// Mudanças de sistema reversíveis: pedem aprovação e criam ponto de restauração.
    Sistema = 2,
    /// Qualquer ação como root: aprovação presencial e concessão de 15 minutos.
    Total = 3,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::Observar => "observar",
            Level::Usuario => "usuario",
            Level::Sistema => "sistema",
            Level::Total => "total",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    Leitura,
    Escreve,
    Destrutiva,
}

/// Regra para um tipo de chamador.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CallerRule {
    pub name: String,
    /// UID do processo que chama (opcional).
    pub uid: Option<u32>,
    /// Prefixo do caminho do executável (opcional).
    pub exe_prefix: Option<String>,
    /// Prefixo do cgroup (opcional) — agentes nativos rodam em `/kalango.slice/kalango-agentes.slice/…`.
    pub cgroup_prefix: Option<String>,
    pub max_level: Level,
    /// Pastas em que o nível usuário pode ler e escrever (`~` = casa do chamador).
    #[serde(default)]
    pub allowed_paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    /// Caminho do soquete.
    pub socket: PathBuf,
    /// Pasta de estado (linha do tempo, aprovações).
    pub state_dir: PathBuf,
    /// Pastas que nenhum agente acessa, em nenhum nível (`~` = casa do chamador).
    pub protected_paths: Vec<String>,
    /// A aprovação precisa vir de um processo ligado a um terminal (presencial).
    pub approval_requires_tty: bool,
    /// Tempo máximo esperando uma aprovação, em segundos.
    pub approval_timeout_s: u64,
    /// Duração da concessão do nível total, em segundos.
    pub total_grant_s: u64,
    /// Não executa ações de nível sistema/total: só descreve o que faria.
    pub dry_run: bool,
    /// Limites de cada tarefa que o kalangod executa para um agente (memória, CPU, processos, tempo).
    #[serde(default)]
    pub limites: crate::limits::Limits,
    /// Raiz cujo /etc entra nos pontos de restauração (só muda em testes).
    #[serde(default = "raiz_padrao")]
    pub restore_root: PathBuf,
    /// Executáveis que podem aprovar sem terminal: a tela de aprovação do sistema.
    #[serde(default = "aprovadores_padrao")]
    pub aprovadores_graficos: Vec<PathBuf>,
    /// Quantos pontos de restauração manter (os mais antigos são apagados).
    #[serde(default = "pontos_padrao")]
    pub pontos_max: usize,
    /// `web.ler` may reach local and private addresses (only for tests with a local page server).
    #[serde(default)]
    pub web_local: bool,
    /// Where `arquivos.lixeira` puts files. Unset (the normal case): the caller's own trash,
    /// `~/.local/share/Trash`. Tests point it at a temporary folder so they never touch a real trash.
    #[serde(default)]
    pub trash_dir: Option<PathBuf>,
    /// The file KalangoOS's niri reads to know who may capture the screen and inject input (patch 0008); its
    /// session runs niri with `KALANGO_NIRI_CONTROLE` pointing here.
    #[serde(default = "controle_remoto_padrao")]
    pub controle_remoto: PathBuf,
    /// System programs that always have a capability: "path capabilities" (the shell's on-screen keyboard types).
    #[serde(default = "controle_remoto_sempre_padrao")]
    pub controle_remoto_sempre: Vec<String>,
    /// The Kalango Remote apps that may ask for a session's permission (for their own core, after the person's
    /// approval).
    #[serde(default = "controle_remoto_apps_padrao")]
    pub controle_remoto_apps: Vec<String>,
    /// The screen helper kalangod starts for an agent's screen-control session (the `tela.*` tools).
    #[serde(default = "tela_auxiliar_padrao")]
    pub tela_auxiliar: PathBuf,
    /// The most actions one screen-control session may take, and the longest it may last (minutes).
    #[serde(default = "tela_max_acoes_padrao")]
    pub tela_max_acoes: u32,
    #[serde(default = "tela_minutos_max_padrao")]
    pub tela_minutos_max: u64,
    pub callers: Vec<CallerRule>,
}

fn tela_auxiliar_padrao() -> PathBuf {
    "/usr/libexec/kalango/kalango-tela".into()
}

fn tela_max_acoes_padrao() -> u32 {
    100
}

fn tela_minutos_max_padrao() -> u64 {
    30
}

fn controle_remoto_padrao() -> PathBuf {
    "/run/kalango/controle-remoto".into()
}

fn controle_remoto_sempre_padrao() -> Vec<String> {
    vec!["/usr/bin/kalango-shell teclado".into()]
}

fn controle_remoto_apps_padrao() -> Vec<String> {
    vec!["/usr/bin/kalango-remote".into()]
}

fn raiz_padrao() -> PathBuf {
    "/".into()
}

fn aprovadores_padrao() -> Vec<PathBuf> {
    vec!["/usr/libexec/kalango/kalango-aprovacao".into()]
}

fn pontos_padrao() -> usize {
    20
}

/// What KalangoOS keeps for itself in a person's home and no agent reaches, to read or to write: the files that
/// decide what the agents (and other privileged apps) may do. The home is the agents' allowed folder, so without
/// this an agent could write its own permissions (`agente-permissoes.json`), add an MCP connector without the
/// approval `mcp.registrar` asks (`conectores/`), turn on unattended remote access (`remote/`, Kalango Remote's
/// core in `~/.config/kalango-remote`), bind a key to a command in the compositor (`atalhos.kdl`), raise its
/// budget, change the agents platform's agents and approvals (`fluxos/`), mark a person's machine as a throwaway
/// copy that is then deleted without asking (`virtualdesk/`), put words in its own past conversations
/// (`conversas/`) or swap the model it thinks with (`modelos/`). The programs that own these files run outside the
/// agents' cgroup and read them directly. A connector's own data folder (`~/.local/share/kalango/conectores/NAME`)
/// stays reachable: the connector runs in the agents' cgroup and works there.
pub const KALANGO_PROTECTED: &[&str] = &[
    "~/.config/kalango",
    "~/.config/kalango-remote",
    "~/.local/share/kalango/fluxos",
    "~/.local/share/kalango/virtualdesk",
    "~/.local/share/kalango/conversas",
    "~/.local/share/kalango/modelos",
    "~/.local/share/kalango/cofre.json",
    // the agent's memory: read only by the agent runtime itself, never through a tool
    "~/.local/share/kalango/memoria.json",
    "~/.local/share/kalango/memoria.bin",
    // the day's token use (against the budget), the permissions' use and the connectors' call records
    "~/.local/state/kalango",
];

impl Config {
    pub fn system_default() -> Config {
        let mut protected: Vec<String> = ["~/.ssh", "~/.gnupg", "~/.local/share/keyrings"].iter().map(|p| p.to_string()).collect();
        protected.extend(KALANGO_PROTECTED.iter().map(|p| p.to_string()));
        protected.push("/etc/shadow".into());
        Config {
            socket: "/run/kalango/mcp.sock".into(),
            state_dir: "/var/lib/kalango".into(),
            protected_paths: protected,
            approval_requires_tty: true,
            approval_timeout_s: 120,
            total_grant_s: 15 * 60,
            dry_run: false,
            limites: Default::default(),
            restore_root: raiz_padrao(),
            pontos_max: pontos_padrao(),
            web_local: false,
            trash_dir: None,
            controle_remoto: controle_remoto_padrao(),
            controle_remoto_sempre: controle_remoto_sempre_padrao(),
            controle_remoto_apps: controle_remoto_apps_padrao(),
            tela_auxiliar: tela_auxiliar_padrao(),
            tela_max_acoes: tela_max_acoes_padrao(),
            tela_minutos_max: tela_minutos_max_padrao(),
            aprovadores_graficos: aprovadores_padrao(),
            callers: vec![
                CallerRule {
                    name: "agentes-nativos".into(),
                    uid: None,
                    exe_prefix: None,
                    cgroup_prefix: Some("/kalango.slice/kalango-agentes.slice".into()),
                    max_level: Level::Total,
                    allowed_paths: vec!["~".into()],
                },
                CallerRule {
                    name: "usuario-local".into(),
                    uid: None,
                    exe_prefix: None,
                    cgroup_prefix: None,
                    max_level: Level::Total,
                    allowed_paths: vec!["~".into()],
                },
            ],
        }
    }

    /// Configuração para desenvolvimento: roda sem root, estado em `dir`.
    pub fn dev(dir: &Path) -> Config {
        let mut c = Config::system_default();
        c.socket = dir.join("mcp.sock");
        c.state_dir = dir.join("estado");
        // sem root, os pontos de restauração cobrem uma raiz de teste em vez do /etc real
        c.restore_root = dir.join("raiz");
        let _ = std::fs::create_dir_all(c.restore_root.join("etc"));
        c
    }

    pub fn load(path: &Path) -> Result<Config, String> {
        let t = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        toml::from_str(&t).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Expande `~` para a casa do usuário.
pub fn expand(p: &str, home: &Path) -> PathBuf {
    if p == "~" {
        home.to_path_buf()
    } else if let Some(rest) = p.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(p)
    }
}

/// Normaliza sem tocar o disco: resolve `.` e `..` lexicalmente.
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn is_within(path: &Path, base: &Path) -> bool {
    normalize(path).starts_with(normalize(base))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_ordered() {
        assert!(Level::Observar < Level::Usuario && Level::Usuario < Level::Sistema && Level::Sistema < Level::Total);
    }

    #[test]
    fn dotdot_cannot_escape() {
        let home = Path::new("/home/roo");
        assert!(!is_within(Path::new("/home/roo/../root/.ssh"), home));
        assert!(is_within(Path::new("/home/roo/docs/../notas.md"), home));
        assert_eq!(expand("~/.ssh", home), PathBuf::from("/home/roo/.ssh"));
    }
}
