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
    /// The person's own interface (the shell): the only programs that may add or take out the folders agents'
    /// commands write in (`kalango/pastas_adicionar`, `kalango/pastas_remover`). An agent — in the agents' slice or
    /// any other program talking to kalangod — may not.
    #[serde(default = "interface_padrao")]
    pub interface_da_pessoa: Vec<PathBuf>,
    /// Loader variables the person's interface may start with (a development session preloads its layer-shell
    /// library: `LD_PRELOAD=…`); anything else loaded into it, or a tracer, and it is not taken as the person's.
    #[serde(default = "ambiente_da_interface_padrao")]
    pub interface_ambiente_permitido: Vec<String>,
    /// Loader variables Kalango Remote's app and core may run with: the image starts them through a script that sets
    /// `LD_LIBRARY_PATH` to the libraries beside them. Anything else loaded into them, or a tracer, and no
    /// permission is given (an agent or any program of the same user could otherwise run the app with its own library).
    #[serde(default = "ambiente_do_remote_padrao")]
    pub controle_remoto_ambiente_permitido: Vec<String>,
    /// Folders the executable code of the Remote's app and core may be mapped from (checked as root, from
    /// /proc/<pid>/maps: a library that hid its own `LD_PRELOAD` string is still there).
    #[serde(default = "bibliotecas_do_remote_padrao")]
    pub controle_remoto_bibliotecas: Vec<String>,
    /// The agents' engine, by full path: the only program whose confidence marks are believed.
    #[serde(default = "motor_do_agente_padrao")]
    pub motor_do_agente: Vec<PathBuf>,
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
    /// Files and folders whose contents run at the person's next login or terminal (`~` = the caller's home): an
    /// agent's change to them asks every time, unless the person chose otherwise (see `startup.rs`).
    #[serde(default = "startup_paths_default")]
    pub startup_paths: Vec<String>,
    /// Where the person's choice for those files is kept (`~` = the caller's home): a file under a protected folder,
    /// which no agent reaches. Its key `arquivos_de_inicio` is "perguntar" (the default) or "livre".
    #[serde(default = "startup_setting_default")]
    pub startup_setting: String,
    /// Test sessions only: the home (`~`) of every caller, in place of the one from passwd, so that what an agent
    /// writes to `~/.bashrc` lands in the test's folder and the person's real home stays out of reach.
    #[serde(default)]
    pub home: Option<PathBuf>,
    /// Tests only: the person's login PATH (else read from their session's processes, `startup::login_path_dirs`).
    #[serde(default)]
    pub login_path: Option<String>,
    pub callers: Vec<CallerRule>,
}

fn startup_paths_default() -> Vec<String> {
    STARTUP_PATHS.iter().chain(PROGRAM_FOLDERS).map(|p| p.to_string()).collect()
}

fn startup_setting_default() -> String {
    "~/.config/kalango/agente-permissoes.json".into()
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

/// The kernel names a program by the file it runs, not by the script that started it: on the image `/usr/bin/kalango-shell`
/// is a script that preloads the layer-shell library and execs `/usr/libexec/kalango/kalango-shell.bin`, and niri
/// compares this path with `/proc/pid/exe`. With the script's path the on-screen keyboard was never allowed.
fn controle_remoto_sempre_padrao() -> Vec<String> {
    vec!["/usr/libexec/kalango/kalango-shell.bin teclado".into()]
}

/// What the image's `kalango-shell` and `kalango-aprovacao` scripts set before exec (imagem/construir.sh).
fn ambiente_da_interface_padrao() -> Vec<String> {
    vec!["LD_PRELOAD=libgtk4-layer-shell.so.0".into()]
}

/// What the image's `kalango-remote` script sets before exec (imagem/construir.sh).
fn ambiente_do_remote_padrao() -> Vec<String> {
    vec!["LD_LIBRARY_PATH=/usr/lib/kalango/remote".into()]
}

fn bibliotecas_do_remote_padrao() -> Vec<String> {
    vec!["/usr/lib/".into(), "/lib/".into()]
}

fn motor_do_agente_padrao() -> Vec<PathBuf> {
    vec!["/usr/libexec/kalango/kalango-agente".into()]
}

/// The ISO installs the app in /usr/lib/kalango/remote and /usr/bin/kalango-remote is a script that execs it with the
/// libraries beside it: the kernel then names the app by the second path, so a list with only the first never
/// matched and no session on an installed system could show its screen.
fn controle_remoto_apps_padrao() -> Vec<String> {
    vec!["/usr/bin/kalango-remote".into(), "/usr/lib/kalango/remote/kalango-remote".into()]
}

fn raiz_padrao() -> PathBuf {
    "/".into()
}

/// The image's `/usr/libexec/kalango/kalango-shell` and `kalango-aprovacao` are links to scripts in /usr/bin; the
/// programs themselves are the `.bin` files, and those are what `/proc/pid/exe` (and so the kernel's answer about the
/// caller) names. Listing the links made `is_person_ui` and the graphical approval refuse the real shell and the real
/// approval screen.
fn interface_padrao() -> Vec<PathBuf> {
    vec!["/usr/libexec/kalango/kalango-shell.bin".into()]
}

fn aprovadores_padrao() -> Vec<PathBuf> {
    vec!["/usr/libexec/kalango/kalango-aprovacao.bin".into()]
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
pub const MIND_PROTECTED: &[&str] = &["~/.local/share/kalango/mind", "%t/kalango/mind-confirm"];

pub const KALANGO_PROTECTED: &[&str] = &[
    // the person's Flatpak overrides and settings: a global override (`overrides/global`) stays under every app's own and
    // widens what a creative app that kalangod starts sees (`filesystems=~/.ssh`)
    "~/.local/share/flatpak/overrides",
    "~/.config/flatpak",
    "~/.config/kalango",
    "~/.config/kalango-remote",
    "~/.local/share/kalango/fluxos",
    "~/.local/share/kalango/virtualdesk",
    // VirtualDesk itself, with the "internet only" guard of the sandbox copies inside, and where an older one kept
    // the guard as a separate library: an agent that could swap either would open the copies' network
    "~/.local/bin/kalango-virtualdesk",
    "~/.local/lib/kalango-virtualdesk",
    "~/.local/share/kalango/conversas",
    "~/.local/share/kalango/modelos",
    "~/.local/share/kalango/cofre.json",
    // the agent's memory: read only by the agent runtime itself, never through a tool
    "~/.local/share/kalango/memoria.json",
    "~/.local/share/kalango/memoria.bin",
    // Kalango Mind: its database (graph, text index, evidence) is reached by agents only through the service's tools
    // (a file tool that could read it would go around the scopes, one that could write it could plant "confirmed"
    // knowledge)
    "~/.local/share/kalango/mind",
    // the Mind's confirmation codes (`%t` = the caller's `$XDG_RUNTIME_DIR`, `/run/user/UID`): what the person's own
    // session reads to confirm something an agent asked to remember. An agent that could read one would confirm its
    // own request. The folder is made when the service first needs it, so the rule goes by its place, not by what is
    // there now (`startup::carve` and the guard's probe both handle a folder that does not exist yet)
    "%t/kalango/mind-confirm",
    // the day's token use (against the budget), the permissions' use and the connectors' call records
    "~/.local/state/kalango",
];

/// What runs at the next login, or in every terminal the person opens, from their own home: the shells' startup
/// files (bash, zsh, fish and the plain POSIX `.profile`, and `.bash_aliases`, which Ubuntu's `.bashrc` reads), the
/// X session's (`.xprofile`, `.xsession*`, `.xinitrc`), the environment read at login (`.pam_environment`,
/// `environment.d`, plasma's `env`), the desktop's autostart entries and the person's systemd units (in
/// `~/.config/systemd`, and in `~/.local/share/systemd/user`, which systemd also reads). Something an agent writes
/// there runs as the person later, outside every rule the agent runs under: so a change to them asks, and shows
/// exactly what changes. (The compositor's own files are in `~/.config/kalango`, which is protected.)
pub const STARTUP_PATHS: &[&str] = &[
    "~/.bashrc", "~/.bash_profile", "~/.bash_login", "~/.bash_logout", "~/.bash_aliases", "~/.profile",
    "~/.zshrc", "~/.zshenv", "~/.zprofile", "~/.zlogin", "~/.zlogout",
    "~/.xprofile", "~/.xsession", "~/.xsessionrc", "~/.xinitrc", "~/.pam_environment",
    "~/.config/autostart", "~/.config/autostart-scripts", "~/.config/plasma-workspace/env",
    "~/.config/systemd", "~/.config/environment.d", "~/.config/fish",
    // what those files bring in: Ubuntu's .bashrc sources ~/.bashrc.d (where it exists), zsh reads from ZDOTDIR
    // (~/.config/zsh is where it usually points) and oh-my-zsh loads everything in its custom folder
    "~/.bashrc.d", "~/.config/zsh", "~/.oh-my-zsh/custom",
    // what systemd and D-Bus start for the person: user units (both folders), user tmpfiles, and D-Bus activation
    // (a .service file there names a program that D-Bus runs when anything asks for that name)
    "~/.local/share/systemd", "~/.config/user-tmpfiles.d", "~/.local/share/dbus-1/services",
    // where the person's folders are (Documents, Downloads…): kalangod and the apps write there trusting it
    "~/.config/user-dirs.dirs",
];

/// The folders of programs in the person's PATH ahead of the system's: a program written there under a common name
/// ("ls", "sudo") runs the next time they type it — as much a startup file as .bashrc. Ubuntu's .profile puts
/// ~/.local/bin and ~/bin first; rustup, bun and npm add theirs; and the tool managers' (nvm, conda, pyenv, fnm…) are
/// first in every terminal that loads them ("*": each version). A floor under what the person's PATH shows
/// (`startup::login_path_dirs`). Installing a tool there (pip --user, a release binary, `conda install`) asks too, or
/// goes ahead with "Permitir automaticamente". These folders often hold hard links by design (conda's are its
/// package cache's files), so a hard link in them does not stop commands (`startup::hardlinked`).
pub const PROGRAM_FOLDERS: &[&str] = &[
    "~/.local/bin", "~/.cargo/bin", "~/bin", "~/.bun/bin", "~/.npm-global/bin",
    "~/.nvm/versions/node/*/bin", "~/miniconda3/bin", "~/miniconda3/condabin", "~/anaconda3/bin", "~/anaconda3/condabin",
    "~/.pyenv/shims", "~/.pyenv/bin", "~/.rbenv/shims", "~/.rbenv/bin", "~/.local/share/fnm", "~/.fnm", "~/.volta/bin",
    "~/.deno/bin", "~/go/bin", "~/.opencode/bin", "~/.lmstudio/bin", "~/.sdkman/candidates/*/current/bin",
    "~/.local/share/pnpm", "~/.yarn/bin", "~/.dotnet/tools", "~/.rustup/toolchains/*/bin",
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
            startup_paths: startup_paths_default(),
            startup_setting: startup_setting_default(),
            home: None,
            login_path: None,
            aprovadores_graficos: aprovadores_padrao(),
            interface_da_pessoa: interface_padrao(),
            interface_ambiente_permitido: ambiente_da_interface_padrao(),
            controle_remoto_ambiente_permitido: ambiente_do_remote_padrao(),
            controle_remoto_bibliotecas: bibliotecas_do_remote_padrao(),
            motor_do_agente: motor_do_agente_padrao(),
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
        let mut cfg: Config = toml::from_str(&t).map_err(|e| format!("{}: {e}", path.display()))?;
        cfg.add_always_protected();
        Ok(cfg)
    }

    /// The Mind's folders are always protected, whatever an older `/etc/kalango/kalangod.toml` lists (the file
    /// replaces the default list, and the installer does not update it): a union, not a replacement.
    pub fn add_always_protected(&mut self) {
        for p in MIND_PROTECTED {
            if !self.protected_paths.iter().any(|x| x == p) {
                eprintln!("kalangod: {}", kalango_i18n::trf("the configuration did not list {path} as protected: added", &[("path", p.to_string())]));
                self.protected_paths.push(p.to_string());
            }
        }
    }
}

/// O cgroup de um chamador está sob `prefix`? Um agente pode estar no cgroup do sistema
/// (`/kalango.slice/kalango-agentes.slice/…`) ou sob o gerenciador de usuário do systemd
/// (`/user.slice/user-UID.slice/user@UID.service/kalango.slice/kalango-agentes.slice/…`: o serviço `kalango-fluxosd` e as
/// tarefas que `systemd-run --user` cria): os dois valem como o mesmo cgroup dos agentes.
pub fn cgroup_matches(cgroup: &str, prefix: &str) -> bool {
    if cgroup.starts_with(prefix) {
        return true;
    }
    // (só o gerenciador do usuário do próprio systemd: /user.slice/user-N.slice/user@N.service + o caminho)
    let Some(rest) = cgroup.strip_prefix("/user.slice/user-") else { return false };
    let Some((uid, rest)) = rest.split_once(".slice/user@") else { return false };
    let Some((uid2, tail)) = rest.split_once(".service") else { return false };
    !uid.is_empty() && uid.chars().all(|c| c.is_ascii_digit()) && uid == uid2 && tail.starts_with(prefix)
}

/// Expande `~` para a casa do usuário.
pub fn expand(p: &str, home: &Path) -> PathBuf {
    if p == "~" {
        home.to_path_buf()
    } else if let Some(rest) = p.strip_prefix("~/") {
        home.join(rest)
    } else if let Some(rest) = p.strip_prefix("%t/") {
        runtime_dir(runtime_uid(home)).join(rest)
    } else {
        PathBuf::from(p)
    }
}

/// The runtime folder of a user (`$XDG_RUNTIME_DIR`): `/run/user/UID`, which logind makes for every session. The base
/// is `KALANGOD_RUNTIME_BASE` for the test sessions (kalangod's own environment: an agent never sets it).
pub fn runtime_dir(uid: u32) -> PathBuf {
    static BASE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BASE.get_or_init(|| std::env::var_os("KALANGOD_RUNTIME_BASE").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| "/run/user".into()))
        .join(uid.to_string())
}

/// Whose runtime folder `%t` means for a caller's home: the home's owner (what the passwd entry's home is by definition),
/// else the running user.
fn runtime_uid(home: &Path) -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(home).map(|m| m.uid()).unwrap_or_else(|_| nix::unistd::getuid().as_raw())
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
    #[test]
    fn the_agents_cgroup_is_the_systems_or_the_user_managers() {
        let p = "/kalango.slice/kalango-agentes.slice";
        assert!(super::cgroup_matches("/kalango.slice/kalango-agentes.slice/t.scope", p));
        assert!(super::cgroup_matches("/user.slice/user-1000.slice/user@1000.service/kalango.slice/kalango-agentes.slice/kalango-fluxos.service", p));
        assert!(super::cgroup_matches("/user.slice/user-1000.slice/user@1000.service/kalango.slice/kalango-agentes.slice", p));
        // not a person's own session, another user's manager under a different uid, nor a look-alike name
        assert!(!super::cgroup_matches("/user.slice/user-1000.slice/session-3.scope", p));
        assert!(!super::cgroup_matches("/user.slice/user-1000.slice/user@1001.service/kalango.slice/kalango-agentes.slice/x", p));
        assert!(!super::cgroup_matches("/user.slice/user-1000.slice/user@1000.service/app.slice/kalango.slice/kalango-agentes.slice/x", p));
        assert!(!super::cgroup_matches("/system.slice/kalango.slice/kalango-agentes.slice/x", p));
        assert!(!super::cgroup_matches("/user.slice/user-.slice/user@.service/kalango.slice/kalango-agentes.slice", p));
    }

    use super::*;

    /// The Mind's confirmation codes are in the protected list (the kernel guard, Landlock and the file tools all start from
    /// it), in the installed configuration file too (which replaces the default list, it does not add to it), and `%t`
    /// is the runtime folder of the home's owner.
    #[test]
    fn the_mind_confirmation_folder_is_protected_by_default_and_in_the_installed_config() {
        for want in ["%t/kalango/mind-confirm", "~/.local/share/kalango/mind"] {
            assert!(Config::system_default().protected_paths.iter().any(|p| p == want), "{want} is missing from the default list");
            let installed = Config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("dist/kalangod.toml")).unwrap();
            assert!(installed.protected_paths.iter().any(|p| p == want), "{want} is missing from dist/kalangod.toml");
        }
        let home = tempfile::tempdir().unwrap();
        let uid = nix::unistd::getuid().as_raw();
        assert_eq!(expand("%t/kalango/mind-confirm", home.path()), runtime_dir(uid).join("kalango/mind-confirm"));
        assert!(runtime_dir(uid).ends_with(uid.to_string()));
        // (what is not `%t/` stays as written)
        assert_eq!(expand("%tmp/x", home.path()), PathBuf::from("%tmp/x"));
    }

    /// An installed configuration from before the Mind (no lines for it) is still protected: the load adds them.
    #[test]
    fn an_old_configuration_file_still_protects_the_mind_folders() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("kalangod.toml");
        // (the installed file as it was before the Mind: without the two lines)
        let installed = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("dist/kalangod.toml")).unwrap();
        let old = installed.replace("\"~/.local/share/kalango/mind\", \"%t/kalango/mind-confirm\",", "");
        assert_ne!(old, installed);
        std::fs::write(&f, old).unwrap();
        let cfg = Config::load(&f).unwrap();
        for want in MIND_PROTECTED {
            assert!(cfg.protected_paths.iter().any(|p| p == want), "{want}");
        }
        assert!(cfg.protected_paths.iter().any(|p| p == "~/.ssh"));
        // (and once, when the file already has them)
        let mut again = cfg.clone();
        again.add_always_protected();
        assert_eq!(again.protected_paths, cfg.protected_paths);
    }

    #[test]
    fn levels_are_ordered() {
        assert!(Level::Observar < Level::Usuario && Level::Usuario < Level::Sistema && Level::Sistema < Level::Total);
    }

    #[test]
    fn the_installed_remote_app_may_ask_for_the_screen() {
        // the app as the kernel sees it on the ISO (the wrapper in /usr/bin execs this binary)
        let cfg = Config::system_default();
        assert!(cfg.controle_remoto_apps.iter().any(|a| a == "/usr/lib/kalango/remote/kalango-remote"));
        assert!(cfg.controle_remoto_apps.iter().any(|a| a == "/usr/bin/kalango-remote"));
    }

    #[test]
    fn the_defaults_name_the_programs_the_kernel_reports() {
        let cfg = Config::system_default();
        // the file the image's scripts exec (imagem/construir.sh: `$b.bin`), not the scripts or links to them
        assert_eq!(cfg.interface_da_pessoa, vec![PathBuf::from("/usr/libexec/kalango/kalango-shell.bin")]);
        assert_eq!(cfg.aprovadores_graficos, vec![PathBuf::from("/usr/libexec/kalango/kalango-aprovacao.bin")]);
        assert_eq!(cfg.controle_remoto_sempre, vec!["/usr/libexec/kalango/kalango-shell.bin teclado".to_string()]);
        // and the loader variables those scripts set are allowed, exactly
        assert_eq!(cfg.interface_ambiente_permitido, vec!["LD_PRELOAD=libgtk4-layer-shell.so.0".to_string()]);
        assert_eq!(cfg.controle_remoto_ambiente_permitido, vec!["LD_LIBRARY_PATH=/usr/lib/kalango/remote".to_string()]);
        // the image build still makes exactly these wrappers
        let build = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../imagem/construir.sh")).unwrap();
        assert!(build.contains("LD_PRELOAD=libgtk4-layer-shell.so.0 /usr/libexec/kalango/%s.bin"), "construir.sh changed the wrapper: update the defaults");
        // the Remote's script drops what the shell that launched it preloaded (the shell's own LD_PRELOAD would reach it
        // from the dock and kalangod would refuse it) and sets only the library folder
        assert!(build.contains("exec env -u LD_PRELOAD -u LD_AUDIT LD_LIBRARY_PATH=/usr/lib/kalango/remote /usr/lib/kalango/remote/kalango-remote"), "construir.sh: the Remote's script must unset LD_PRELOAD and LD_AUDIT");
        assert_eq!(cfg.controle_remoto_bibliotecas, vec!["/usr/lib/".to_string(), "/lib/".to_string()]);
    }

    #[test]
    fn dotdot_cannot_escape() {
        let home = Path::new("/home/roo");
        assert!(!is_within(Path::new("/home/roo/../root/.ssh"), home));
        assert!(is_within(Path::new("/home/roo/docs/../notas.md"), home));
        assert_eq!(expand("~/.ssh", home), PathBuf::from("/home/roo/.ssh"));
    }
}
