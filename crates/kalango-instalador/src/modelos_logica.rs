//! The "AI models" step's rules, without any widget: what is ticked, what each source says about each model, what
//! fits on the disk, what the banners say, and the choice the installer records. Tested below.

use kalango_modelos::catalog::{ModelEntry, DEFAULT_CHOSEN, SLOTS};
use kalango_modelos::config::{Config, SourceKind};
use kalango_modelos::probe::{ModelState, Probe};
use kalango_modelos::urlpolicy::{is_loopback_host, is_private_host, parse_server, UrlError};

/// What the installed system takes on the disk, before any model (the live system's size, roughly).
pub const SYSTEM_FOOTPRINT: u64 = 8_000_000_000;

/// Why a test of the source failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fail {
    Unreachable,
    Timeout,
    Denied(u16),
    Offline,
    /// the server answered with another HTTP error
    Http(u16),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Test {
    Idle,
    Running,
    Done(Probe),
    Failed(Fail),
}

/// The little label at the end of a model's row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip {
    None,
    Verifying,
    NoNetwork,
    /// "not verified" (the test failed)
    NotVerified,
    OnServerOk,
    OnServer,
    AbsentOnServer,
    InFolderOk,
    InFolder,
    NotInFolder,
    SumMismatch,
    WrongSize,
    NoChecksum,
}

impl Chip {
    /// good (green), warn (amber), bad (red), neutral (grey)
    pub fn tone(self) -> &'static str {
        match self {
            Chip::OnServerOk | Chip::InFolderOk => "bom",
            Chip::AbsentOnServer | Chip::NotInFolder | Chip::NoChecksum => "aviso",
            Chip::SumMismatch | Chip::WrongSize => "ruim",
            _ => "neutro",
        }
    }
}

/// The banner of the source's own area (under the source cards or the fields).
#[derive(Clone, Debug, PartialEq)]
pub enum Notice {
    None,
    /// the public catalog: size and sum are checked at each download
    Public,
    PublicOffline,
    Later,
    Testing { host: String },
    Connected { host: String, found: usize, of: usize, all_sums: bool },
    /// a file's sum is not the catalog's
    SumMismatch { model: String },
    Unreachable { host: String, timeout: bool },
    Denied { code: u16 },
    /// the server is there but answers with an error
    ServerError { host: String, code: u16 },
    FolderOk { found: usize, of: usize, all_sums: bool },
    FolderEmpty,
}

pub struct State {
    pub catalog: Vec<ModelEntry>,
    pub source: SourceKind,
    pub server: String,
    pub lan_ok: bool,
    pub folder: String,
    pub ticked: Vec<bool>,
    pub test: Test,
    pub online: bool,
    /// bytes available for the system and the models (the disk, or the part given to KalangoOS)
    pub capacity: u64,
    pub footprint: u64,
}

impl State {
    /// `catalog`: the whole catalog; only the five kinds the installer offers are kept.
    pub fn new(catalog: Vec<ModelEntry>, capacity: u64, online: bool) -> State {
        let catalog: Vec<ModelEntry> = catalog.into_iter().filter(|m| SLOTS.iter().any(|(_, id)| *id == m.id)).collect();
        let ticked = SLOTS.iter().map(|(_, id)| DEFAULT_CHOSEN.contains(id)).collect();
        State { catalog, source: SourceKind::Padrao, server: String::new(), lan_ok: false, folder: String::new(), ticked, test: Test::Idle, online, capacity, footprint: SYSTEM_FOOTPRINT }
    }

    pub fn entry(&self, i: usize) -> Option<&ModelEntry> {
        let id = SLOTS.get(i)?.1;
        self.catalog.iter().find(|m| m.id == id)
    }

    /// What the source says about row `i`, once a test ran.
    fn probed(&self, i: usize) -> Option<ModelState> {
        match (&self.test, SLOTS.get(i)) {
            (Test::Done(p), Some((_, id))) => p.state(id),
            _ => None,
        }
    }

    /// Can it be ticked? A model the source does not have, or that it refused, cannot.
    pub fn tickable(&self, i: usize) -> bool {
        if self.source == SourceKind::Depois || self.entry(i).is_none() {
            return false;
        }
        match self.source {
            SourceKind::Servidor | SourceKind::Pasta => self.probed(i).map(ModelState::usable).unwrap_or(true),
            _ => true,
        }
    }

    pub fn effective(&self, i: usize) -> bool {
        self.ticked.get(i).copied().unwrap_or(false) && self.tickable(i)
    }

    pub fn chip(&self, i: usize) -> Chip {
        let (server, folder) = (self.source == SourceKind::Servidor, self.source == SourceKind::Pasta);
        match (&self.test, self.source) {
            (_, SourceKind::Depois) => Chip::None,
            (_, SourceKind::Padrao) => if self.online { Chip::None } else { Chip::NoNetwork },
            (Test::Idle, _) => Chip::None,
            (Test::Running, _) => Chip::Verifying,
            (Test::Failed(_), _) => Chip::NotVerified,
            (Test::Done(_), _) => match (self.probed(i), server, folder) {
                (Some(ModelState::Verified), true, _) => Chip::OnServerOk,
                (Some(ModelState::Unverified), true, _) => Chip::OnServer,
                (Some(ModelState::Absent), true, _) => Chip::AbsentOnServer,
                (Some(ModelState::Verified), _, true) => Chip::InFolderOk,
                (Some(ModelState::Unverified), _, true) => Chip::InFolder,
                (Some(ModelState::Absent), _, true) => Chip::NotInFolder,
                (Some(ModelState::SumMismatch), _, _) => Chip::SumMismatch,
                (Some(ModelState::WrongSize), _, _) => Chip::WrongSize,
                (Some(ModelState::NoChecksum), _, _) => Chip::NoChecksum,
                _ => Chip::None,
            },
        }
    }

    /// A test's result: the models the source cannot give are unticked.
    pub fn set_test(&mut self, t: Test) {
        self.test = t;
        for i in 0..self.ticked.len() {
            if !self.tickable(i) {
                self.ticked[i] = false;
            }
        }
    }

    /// The source or its address changed: what was tested no longer applies.
    pub fn invalidate(&mut self) {
        self.test = Test::Idle;
    }

    pub fn total(&self) -> u64 {
        (0..self.ticked.len()).filter(|i| self.effective(*i)).filter_map(|i| self.entry(i)).map(ModelEntry::bytes).sum()
    }

    /// Free bytes after the system and the models (negative: it does not fit).
    pub fn free_after(&self) -> i64 {
        self.capacity as i64 - self.footprint as i64 - self.total() as i64
    }

    pub fn missing(&self) -> u64 {
        (-self.free_after()).max(0) as u64
    }

    pub fn enough_space(&self) -> bool {
        self.free_after() >= 0
    }

    pub fn server_error(&self) -> Option<UrlError> {
        if self.server.trim().is_empty() {
            return None;
        }
        parse_server(&self.server, self.lan_ok).err()
    }

    /// Does the address need the "private network" confirmation (plain http to a private, non-loopback host)?
    pub fn needs_lan_confirmation(&self) -> bool {
        url_host(&self.server).is_some_and(|(plain, h)| plain && !is_loopback_host(&h) && is_private_host(&h))
    }

    pub fn server_valid(&self) -> bool {
        !self.server.trim().is_empty() && self.server_error().is_none()
    }

    pub fn can_continue(&self) -> bool {
        match self.source {
            SourceKind::Padrao => self.enough_space(),
            SourceKind::Servidor => self.server_valid() && self.enough_space(),
            SourceKind::Pasta => self.folder.starts_with('/') && self.enough_space(),
            SourceKind::Depois => true,
        }
    }

    pub fn chosen_ids(&self) -> Vec<String> {
        SLOTS.iter().enumerate().filter(|(i, _)| self.effective(*i)).map(|(_, (_, id))| id.to_string()).collect()
    }

    /// What the installer records in the installed system.
    pub fn config(&self) -> Config {
        Config {
            fonte: self.source,
            servidor: (self.source == SourceKind::Servidor).then(|| self.server.trim().trim_end_matches('/').to_string()),
            pasta: (self.source == SourceKind::Pasta).then(|| self.folder.clone()),
            escolhidos: self.chosen_ids(),
            rede_local_confirmada: self.source == SourceKind::Servidor && self.lan_ok && self.needs_lan_confirmation(),
        }
    }

    pub fn host(&self) -> String {
        parse_server(&self.server, self.lan_ok).map(|s| s.host().to_string()).unwrap_or_else(|_| url_host(&self.server).map(|(_, h)| h).unwrap_or_default())
    }

    pub fn notice(&self) -> Notice {
        match (self.source, &self.test) {
            (SourceKind::Padrao, _) => if self.online { Notice::Public } else { Notice::PublicOffline },
            (SourceKind::Depois, _) => Notice::Later,
            (SourceKind::Servidor, Test::Idle) | (SourceKind::Pasta, Test::Idle) => Notice::None,
            (SourceKind::Servidor, Test::Running) => Notice::Testing { host: self.host() },
            (SourceKind::Pasta, Test::Running) => Notice::Testing { host: String::new() },
            (_, Test::Failed(Fail::Denied(c))) => Notice::Denied { code: *c },
            (_, Test::Failed(Fail::Offline)) => Notice::PublicOffline,
            (_, Test::Failed(Fail::Http(c))) => Notice::ServerError { host: self.host(), code: *c },
            (_, Test::Failed(f)) => Notice::Unreachable { host: self.host(), timeout: *f == Fail::Timeout },
            (src, Test::Done(p)) => {
                if let Some(i) = (0..SLOTS.len()).find(|i| matches!(self.probed(*i), Some(ModelState::SumMismatch))) {
                    return Notice::SumMismatch { model: self.entry(i).map(|m| m.nome.clone()).unwrap_or_default() };
                }
                let (found, of) = (p.found(), SLOTS.len());
                if src == SourceKind::Pasta {
                    if found == 0 { Notice::FolderEmpty } else { Notice::FolderOk { found, of, all_sums: p.all_verified() } }
                } else {
                    Notice::Connected { host: self.host(), found, of, all_sums: p.all_verified() }
                }
            }
        }
    }
}

/// (is it plain http, the host) of an address typed by the person.
fn url_host(s: &str) -> Option<(bool, String)> {
    let u = url::Url::parse(s.trim()).ok()?;
    Some((u.scheme() == "http", u.host_str()?.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kalango_modelos::catalog::{parse_catalog, ModelEntry};

    fn catalog() -> Vec<ModelEntry> {
        parse_catalog(include_str!("../../../sessao/dist/share/kalango/loja/modelos.json")).unwrap()
    }

    fn state() -> State {
        State::new(catalog(), 600_000_000_000, true)
    }

    fn probe(states: &[(&str, ModelState)]) -> Probe {
        Probe { models: states.iter().map(|(i, s)| (i.to_string(), *s)).collect(), sums_found: true }
    }

    #[test]
    fn the_first_choice_is_text_voice_and_vision_from_the_public_source() {
        let s = state();
        assert_eq!(s.source, SourceKind::Padrao);
        assert_eq!(s.chosen_ids(), ["lfm3-1.7b", "lfm3-audio-2b", "lfm3-vl-2b"]);
        assert!((3_700_000_000..4_000_000_000).contains(&s.total()), "{}", s.total());
        assert!(s.can_continue());
        assert_eq!(s.config(), Config::default());
    }

    #[test]
    fn text_can_be_unticked() {
        let mut s = state();
        s.ticked[0] = false;
        assert_eq!(s.chosen_ids(), ["lfm3-audio-2b", "lfm3-vl-2b"]);
    }

    #[test]
    fn later_records_nothing_to_download_and_always_continues() {
        let mut s = state();
        s.source = SourceKind::Depois;
        assert_eq!(s.total(), 0);
        assert!(s.chosen_ids().is_empty() && s.can_continue());
        assert!(!s.tickable(0));
        assert_eq!(s.config().fonte, SourceKind::Depois);
        assert_eq!(s.notice(), Notice::Later);
    }

    #[test]
    fn a_refused_model_is_unticked_and_cannot_be_ticked() {
        let mut s = state();
        s.source = SourceKind::Servidor;
        s.server = "https://modelos.example.org/kalango".into();
        s.set_test(Test::Done(probe(&[("lfm3-1.7b", ModelState::Verified), ("lfm3-audio-2b", ModelState::Verified), ("lfm3-vl-2b", ModelState::SumMismatch), ("lfm3-omni-2b", ModelState::Absent), ("d1-2b", ModelState::Verified)])));
        assert!(!s.ticked[2], "vision was refused: unticked");
        assert!(!s.tickable(2) && !s.tickable(3) && s.tickable(4));
        s.ticked[2] = true; // even if the widget were forced on
        assert!(!s.effective(2));
        assert_eq!(s.chosen_ids(), ["lfm3-1.7b", "lfm3-audio-2b"]);
        assert_eq!(s.chip(2), Chip::SumMismatch);
        assert_eq!(s.chip(3), Chip::AbsentOnServer);
        assert_eq!(s.chip(0), Chip::OnServerOk);
        assert_eq!(s.notice(), Notice::SumMismatch { model: "LFM3 VL 2B".into() });
        // the total counts only what will be downloaded (design 07: 2,6 GB)
        assert!((2_400_000_000..2_700_000_000).contains(&s.total()));
    }

    #[test]
    fn the_server_notices_follow_the_test() {
        let mut s = state();
        s.source = SourceKind::Servidor;
        s.server = "https://modelos.example.org/kalango".into();
        assert_eq!(s.notice(), Notice::None);
        assert_eq!(s.chip(0), Chip::None);
        s.set_test(Test::Running);
        assert_eq!(s.notice(), Notice::Testing { host: "modelos.example.org".into() });
        assert_eq!(s.chip(1), Chip::Verifying);
        s.set_test(Test::Failed(Fail::Timeout));
        assert_eq!(s.notice(), Notice::Unreachable { host: "modelos.example.org".into(), timeout: true });
        assert_eq!(s.chip(0), Chip::NotVerified);
        s.set_test(Test::Failed(Fail::Denied(401)));
        assert_eq!(s.notice(), Notice::Denied { code: 401 });
        s.set_test(Test::Failed(Fail::Http(503)));
        assert_eq!(s.notice(), Notice::ServerError { host: "modelos.example.org".into(), code: 503 });
        s.set_test(Test::Done(probe(&[("lfm3-1.7b", ModelState::Verified), ("lfm3-audio-2b", ModelState::Verified), ("lfm3-vl-2b", ModelState::Verified), ("lfm3-omni-2b", ModelState::Absent), ("d1-2b", ModelState::Verified)])));
        assert_eq!(s.notice(), Notice::Connected { host: "modelos.example.org".into(), found: 4, of: 5, all_sums: true });
        s.invalidate();
        assert_eq!(s.test, Test::Idle);
    }

    #[test]
    fn space_is_counted_on_the_disk_and_blocks_continuing() {
        let mut s = State::new(catalog(), 12_000_000_000, true);
        s.ticked = vec![true; 5];
        let need = s.footprint + s.total();
        assert!(need > 12_000_000_000);
        assert!(!s.enough_space() && !s.can_continue());
        assert_eq!(s.missing(), need - 12_000_000_000);
        s.ticked = vec![true, false, false, false, false];
        assert!(s.can_continue());
    }

    #[test]
    fn the_server_address_rules_show_in_the_state() {
        let mut s = state();
        s.source = SourceKind::Servidor;
        assert!(!s.server_valid() && !s.can_continue());
        s.server = "http://modelos.example.org".into();
        assert_eq!(s.server_error(), Some(UrlError::PlainHttp));
        assert!(!s.needs_lan_confirmation(), "a public name over http is refused, not confirmable");
        s.server = "http://192.168.1.5/m".into();
        assert!(s.needs_lan_confirmation() && !s.server_valid());
        s.lan_ok = true;
        assert!(s.server_valid() && s.config().rede_local_confirmada);
        s.server = "http://127.0.0.1:8080/m".into();
        s.lan_ok = false;
        assert!(s.server_valid() && !s.config().rede_local_confirmada);
        s.server = "https://modelos.example.org/kalango/".into();
        assert_eq!(s.config().servidor.as_deref(), Some("https://modelos.example.org/kalango"));
    }

    #[test]
    fn offline_default_source_says_so_but_still_continues() {
        let s = State::new(catalog(), 600_000_000_000, false);
        assert_eq!(s.notice(), Notice::PublicOffline);
        assert_eq!(s.chip(0), Chip::NoNetwork);
        assert!(s.can_continue());
    }

    #[test]
    fn a_folder_needs_an_absolute_path_and_reports_what_it_has() {
        let mut s = state();
        s.source = SourceKind::Pasta;
        assert!(!s.can_continue());
        s.folder = "/media/pendrive/modelos".into();
        assert!(s.can_continue());
        s.set_test(Test::Done(probe(&[("lfm3-1.7b", ModelState::Verified), ("lfm3-audio-2b", ModelState::Verified), ("lfm3-vl-2b", ModelState::Verified), ("lfm3-omni-2b", ModelState::Absent), ("d1-2b", ModelState::Verified)])));
        assert_eq!(s.notice(), Notice::FolderOk { found: 4, of: 5, all_sums: true });
        assert_eq!(s.chip(3), Chip::NotInFolder);
        assert_eq!(s.chip(0), Chip::InFolderOk);
        let c = s.config();
        assert_eq!((c.fonte, c.pasta.as_deref()), (SourceKind::Pasta, Some("/media/pendrive/modelos")));
        assert!(c.servidor.is_none());
    }
}
