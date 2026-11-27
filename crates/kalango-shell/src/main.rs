//! KalangoOS · shell da sessão: barra superior, dock, área de trabalho, barra de comando e menu de energia, num
//! processo só (superfícies layer-shell sobre o niri). Também é o servidor de notificações da sessão. O estado vem do niri (espaços e janelas, pelo IPC) e do
//! kalangod (pedidos de aprovação).
//!
//! Uso:
//!   kalango-shell                 inicia o shell (uma instância por sessão)
//!   kalango-shell comando         abre/fecha a barra de comando (atalho Super+Espaço)
//!   kalango-shell apps            abre a barra de comando com todos os apps
//!   kalango-shell energia         abre/fecha o menu de energia
//!   kalango-shell lateral avisos|controles|agente   barra lateral direita (Super+N, Super+C)
//!   kalango-shell hoje            barra lateral esquerda "Hoje" (Super+W)
//!   kalango-shell terminal-suspenso [expandir]   o Terminal suspenso (F12; Shift+F12 expande)
//!   kalango-shell pedir TEXTO     dá um pedido ao agente da barra lateral
//!   kalango-shell conversar TEXTO uma conversa nova na Central de agentes, com esse pedido
//!   kalango-shell sair            encerra o shell

mod agent_editor;
mod agent_prefs;
mod ajustes_chaves;
mod ajustes_modelos;
mod ajustes_rede;
mod ajustes_sandbox;
mod agent_screen;
mod agent_sandbox;
mod agent_settings;
mod app_store;
mod app_store_data;
mod modelos_fonte;
mod modelos_licenca;
mod apps;
#[allow(dead_code)] // the screens of the agents center use it as they land
mod fluxos;
#[allow(dead_code)] // the API of the states, for the screens of the agents center as they land
mod fluxos_estados;
mod fluxos_ferramentas;
// (some helpers serve the editors of the platform, which come next)
#[allow(dead_code)]
mod fluxos_ui;
mod flow_router;
mod fluxo_canvas;
mod fluxo_construtor;
mod fluxo_execucoes;
mod fluxo_inspetor;
mod fluxo_modelo;
mod arquivos;
mod desktop_files;
mod bloqueio;
mod limite;
mod fences;
mod latex_block;
mod latex_pdf;
mod fit_row;
mod wrap_row;
mod resize_handle;
mod widths;
mod central;
mod central_agentes;
mod central_agentes_modelos;
mod central_ajustes;
mod central_conector_novo;
mod central_conversas;
mod central_linha;
mod central_memoria;
mod central_permissoes;
mod central_rotinas;
mod central_uso;
mod area;
mod comando;
mod configuracoes;
mod configuracoes_aparencia;
mod configuracoes_moldura;
mod configuracoes_rede;
mod configuracoes_bluetooth;
mod configuracoes_tela;
mod configuracoes_som;
mod configuracoes_energia;
mod configuracoes_notificacoes;
mod idle;
mod notification_policy;
mod clipboard;
mod clipboard_watch;
mod criativos;
mod paste_keys;
mod voice_mode;
mod configuracoes_entrada;
mod configuracoes_sistema;
mod configuracoes_acessibilidade;
mod configuracoes_agentes;
mod configuracoes_privacidade;
mod descrever;
mod emoji;
mod osd;
mod osk;
mod alertas;
mod bluez_agent;
mod bt_pairing;
mod nm_agent;
mod polkit_agent;
mod print_dialog;
mod privacy_now;
mod ajustes_tuneis;
mod fluxo_chamadas;
mod fluxo_internet;
mod internet_modelo;
mod tunel_agora;
mod access_dialog;
mod shutdown_screen;
mod syntax;
mod system_sounds;
mod focus_session;
mod screen_share;
mod screenshot_tool;
mod shelf;
mod shortcuts_sheet;
mod island_card;
mod layout_habits;
mod spaces;
mod start_menu;
mod taskbar;
mod window_preview;
mod app_theme;
mod calc_engine;
mod calculadora;
mod editor;
mod image_ops;
mod image_viewer;
mod monitor;
mod music_library;
mod music_player;
mod calendar_app;
mod help_app;
mod disks_app;
mod disks_data;
mod doc_reader;
mod video_media;
mod video_player;
mod monitor_dados;
mod monitor_paginas;
mod terminal;
mod terminal_dropdown;
mod terminal_prefs;
mod ssh_hosts;
mod edges;
mod window_prefs;
mod portal;
mod removable_media;
mod unresponsive;
mod snap_divider;
mod resize_measure;
mod magnifier;
mod html_canvas;
mod wifi_password;
mod legendas;
mod input_settings;
mod layout_audit;
mod shortcuts;
mod agent_icon_picker;
mod agent_structured;
mod chat_blocks;
mod agent_picker;
mod conversa;
mod encaixe;
mod energia;
mod grade;
mod estado;
mod lateral;
mod code_block;
mod context_menu;
mod diagram_block;
mod diff_card;
mod disk_answer;
mod table_block;
mod terminal_block;
mod markdown;
mod mem_debug;
mod window_state;
mod release_policy;
mod math_block;
mod media_block;
mod mem_answer;
mod new_agent;
mod install_agent;
mod agent_browser;
mod midia;
mod notificacoes;
mod operacao;
mod pam;
mod pedido_aviso;
mod rotinas;
mod papel;
mod pastas;
mod primeiro_uso;
mod servicos;
mod sistema;
mod superficies;
mod tema;
mod text_focus;
mod usando;
mod visao;
mod voz;
mod widgets;
mod agenda;
mod alternador;
mod clima;
mod indice;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;


type SystemListener = Box<dyn Fn(&sistema::Sistema)>;

pub struct Shell {
    pub estado: Rc<estado::Estado>,
    pub todos: Vec<apps::App>,
    pub fixados: RefCell<Vec<apps::App>>,
    pub midia: Rc<midia::Players>,
    right_bar: RefCell<Option<Rc<lateral::Direita>>>,
    today_bar: RefCell<Option<Rc<lateral::Hoje>>>,
    /// listeners for real system state (status icons in the top bar)
    system_listeners: RefCell<Vec<SystemListener>>,
    comando: RefCell<Option<Rc<comando::Comando>>>,
    notificacoes: RefCell<Option<Rc<notificacoes::Notificacoes>>>,
    energia: RefCell<Option<Rc<energia::Energia>>>,
    clipboard: RefCell<Option<Rc<clipboard::Clipboard>>>,
    voice: RefCell<Option<Rc<voice_mode::VoiceMode>>>,
    encaixe: RefCell<Option<Rc<encaixe::Encaixe>>>,
    alternador: RefCell<Option<Rc<alternador::Alternador>>>,
    grade: RefCell<Option<Rc<grade::Grade>>>,
    bloqueio: RefCell<Option<Rc<bloqueio::Bloqueio>>>,
    visao: RefCell<Option<Rc<visao::Visao>>>,
    /// the desktop's widgets
    area: RefCell<Option<Rc<widgets::Area>>>,
    /// the agents center (made the first time it is opened)
    central: RefCell<Option<Rc<central::Central>>>,
    arquivos: RefCell<Option<Rc<arquivos::Arquivos>>>,
    configuracoes: RefCell<Option<Rc<configuracoes::Configuracoes>>>,
    /// the routines' scheduler
    agendador: RefCell<Option<Rc<rotinas::Agendador>>>,
    /// "Notificação de aprovação": an agent asks and its conversation is not on screen
    aviso_pedido: RefCell<Option<Rc<pedido_aviso::AvisoPedido>>>,
    usando: RefCell<Option<Rc<usando::AgenteUsando>>>,
    operacoes: RefCell<Option<Rc<operacao::Operacoes>>>,
    /// the dock's window (full-screen overlays leave it out of their scrim)
    pub dock: RefCell<Option<gtk::ApplicationWindow>>,
    pub barra: RefCell<Option<gtk::ApplicationWindow>>,
}

impl Shell {
    /// The desktop's widgets (to add one from elsewhere: an answer pinned as a widget).
    pub fn area_de_trabalho(&self) -> Option<Rc<widgets::Area>> {
        self.area.borrow().clone()
    }

    pub fn comando(&self) -> Rc<comando::Comando> {
        self.comando.borrow().clone().unwrap()
    }
    /// Closes the full-screen overlays that keep the bar and the dock usable (the grid, the overview).
    pub fn fechar_sobreposicoes(&self) {
        if let Some(v) = self.visao.borrow().as_ref().filter(|v| v.aberta.get()) {
            v.fechar(None);
        }
        if let Some(g) = self.grade.borrow().as_ref().filter(|g| g.aberta.get()) {
            g.fechar();
        }
    }
    pub fn aviso_pedido(&self) -> Option<Rc<pedido_aviso::AvisoPedido>> {
        self.aviso_pedido.borrow().clone()
    }
    /// Long file operations (a copy in the island).
    pub fn operacoes(&self) -> Option<Rc<operacao::Operacoes>> {
        self.operacoes.borrow().clone()
    }
    /// The banner "the agent is using …" (and the island's task progress).
    pub fn usando(&self) -> Option<Rc<usando::AgenteUsando>> {
        self.usando.borrow().clone()
    }
    /// Hooks a conversation's requests to the approval notice (they show when the conversation is not on screen).
    pub fn ligar_pedidos(self: &Rc<Self>, conv: &Rc<conversa::Conversa>) {
        let (s, cw) = (Rc::downgrade(self), Rc::downgrade(conv));
        conv.ao_pedir(move |id, pedido| {
            let (Some(s), Some(c)) = (s.upgrade(), cw.upgrade()) else { return };
            let Some(a) = s.aviso_pedido() else { return };
            match pedido {
                Some((titulo, detalhe)) => a.mostrar(&c, id, &titulo, &detalhe),
                None => a.respondido(id),
            }
        });
    }
    pub fn agendador(&self) -> Option<Rc<rotinas::Agendador>> {
        self.agendador.borrow().clone()
    }
    /// The agents center (os.kalango.Agentes).
    pub fn abrir_central(self: &Rc<Self>, pagina: Option<&str>) {
        // an app opens: the overlays over the windows close (a pinned side bar stays, it has its own room)
        self.fechar_sobreposicoes();
        self.right_bar().close_if_floating();
        self.today_bar().close_if_floating();
        let existente = self.central.borrow().clone();
        let c = match existente {
            Some(c) => c,
            None => {
                let Some(app) = gtk::gio::Application::default().and_downcast::<gtk::Application>() else { return };
                let c = central::Central::new(&app, self);
                *self.central.borrow_mut() = Some(c.clone());
                c
            }
        };
        c.abrir();
        if let Some(p) = pagina {
            c.pagina(p);
        }
    }
    pub fn central(&self) -> Option<Rc<central::Central>> {
        self.central.borrow().clone()
    }

    /// A third-party agent's manifest: its install page, permissions first.
    pub fn instalar_agente(self: &Rc<Self>, arquivo: &std::path::Path) {
        self.abrir_central(None);
        if let Some(c) = self.central() {
            c.instalar_agente(arquivo);
        }
    }

    /// KalangoOS's file manager, on a folder (or where it was).
    pub fn abrir_arquivos(self: &Rc<Self>, pasta: Option<&std::path::Path>) {
        self.fechar_sobreposicoes();
        self.right_bar().close_if_floating();
        self.today_bar().close_if_floating();
        let existente = self.arquivos.borrow().clone();
        let a = match existente {
            Some(a) => a,
            None => {
                let Some(app) = gtk::gio::Application::default().and_downcast::<gtk::Application>() else { return };
                let a = arquivos::Arquivos::new(&app, self);
                *self.arquivos.borrow_mut() = Some(a.clone());
                a
            }
        };
        a.abrir_em(pasta);
    }

    /// Files' "Open with" dialog for a file (the desktop's "Abrir com › Outro app…").
    pub fn abrir_com(self: &Rc<Self>, arquivo: &std::path::Path) {
        self.abrir_arquivos(Some(arquivo));
        if let Some(a) = self.arquivos.borrow().clone() {
            a.open_with(arquivo);
        }
    }

    /// KalangoOS's settings, on a section (or the home page).
    pub fn abrir_configuracoes(self: &Rc<Self>, secao: Option<&str>) {
        self.fechar_sobreposicoes();
        self.right_bar().close_if_floating();
        self.today_bar().close_if_floating();
        let existente = self.configuracoes.borrow().clone();
        let c = match existente {
            Some(c) => c,
            None => {
                let Some(app) = gtk::gio::Application::default().and_downcast::<gtk::Application>() else { return };
                let c = configuracoes::Configuracoes::new(&app, self);
                *self.configuracoes.borrow_mut() = Some(c.clone());
                c
            }
        };
        c.abrir(secao);
    }

    /// The settings window, when it was opened.
    pub fn configuracoes_atual(&self) -> Option<Rc<configuracoes::Configuracoes>> {
        self.configuracoes.borrow().clone()
    }

    pub fn visao(&self) -> Rc<visao::Visao> {
        self.visao.borrow().clone().unwrap()
    }
    pub fn bloqueio(&self) -> Rc<bloqueio::Bloqueio> {
        self.bloqueio.borrow().clone().unwrap()
    }
    pub fn grade(&self) -> Rc<grade::Grade> {
        self.grade.borrow().clone().unwrap()
    }
    pub fn energia(&self) -> Rc<energia::Energia> {
        self.energia.borrow().clone().unwrap()
    }
    pub fn notificacoes(&self) -> Rc<notificacoes::Notificacoes> {
        self.notificacoes.borrow().clone().unwrap()
    }
    pub fn right_bar(&self) -> Rc<lateral::Direita> {
        self.right_bar.borrow().clone().unwrap()
    }
    pub fn today_bar(&self) -> Rc<lateral::Hoje> {
        self.today_bar.borrow().clone().unwrap()
    }
    /// Called by the side bar after reading the system; updates whoever listens (top bar icons).
    pub fn estado_sistema(&self, s: &sistema::Sistema) {
        for f in self.system_listeners.borrow().iter() {
            f(s);
        }
    }
    pub fn on_system_state(&self, f: impl Fn(&sistema::Sistema) + 'static) {
        self.system_listeners.borrow_mut().push(Box::new(f));
    }
    pub fn notificar(&self, app: &str, title: &str, body: &str) {
        self.notificacoes().notify_local(app, title, body);
    }
    fn executar(self: &Rc<Self>, app: &gtk::Application, args: &[String]) {
        // (the window tests: when the command came, to time a window opened again until its first frame)
        if std::env::var_os("KALANGO_MEDIR_JANELAS").is_some() && args.first().is_some_and(|a| a != "estado" && a != "memoria") {
            eprintln!("debug medir: comando {} t={}", args.join(" "), glib::monotonic_time());
        }
        match args.first().map(String::as_str) {
            Some("comando") => self.comando().alternar(),
            Some("apps") => self.grade().alternar(),
            Some("bloquear") => self.bloqueio().bloquear(),
            Some("visao") => self.visao().alternar(),
            Some("espacos") => {
                if let Some(s) = spaces::get() {
                    s.alternar();
                }
            }
            Some("central") => self.abrir_central(args.get(1).map(String::as_str)),
            // a flow of kalango-fluxos in the builder (scripts, tests)
            Some("fluxo") if args.len() > 1 => {
                self.abrir_central(None);
                if let Some(c) = self.central() {
                    c.abrir_fluxo(&args[1]);
                }
            }
            Some("instalar-agente") => {
                if let Some(f) = args.get(1) {
                    self.instalar_agente(std::path::Path::new(f));
                }
            }
            Some("widgets") => {
                if let Some(a) = self.area.borrow().clone() {
                    a.editar();
                }
            }
            // "energia desligar|reiniciar": straight to wrapping up (scripts, tests); otherwise the menu
            Some("energia") => match args.get(1).map(String::as_str) {
                Some(a @ ("desligar" | "reiniciar")) => shutdown_screen::start(a, false),
                _ => self.energia().alternar(),
            },
            // the desktop (Super+D): the empty space at the end, and back
            Some("area-de-trabalho") => edges::show_desktop(),
            // the shortcuts sheet (Super+/)
            Some("atalhos") => shortcuts_sheet::toggle(),
            // the magnifier (Super =, Super −, Super 0): "lupa mais|menos|sair|modo lente|tela|metade"
            Some("lupa") => magnifier::command(args.get(1).map(String::as_str), args.get(2).map(String::as_str)),
            // the screenshot tool (Print); while a video records, it stops it
            Some("captura") => screenshot_tool::open(args.get(1).map(String::as_str) == Some("video")),
            // focus mode: "foco" starts or pauses the timer (the Today ring's click), "foco painel" opens the session's panel
            Some("foco") => {
                if args.get(1).map(String::as_str) == Some("painel") {
                    focus_session::toggle_panel();
                } else {
                    self.today_bar().foco_alternar();
                }
            }
            // the top bar's privacy: "pausar" cuts every sensor and pauses the agents (Super+Shift+P), else the panel
            Some("privacidade") => {
                if args.get(1).map(String::as_str) == Some("pausar") {
                    privacy_now::pause_all();
                } else {
                    privacy_now::toggle_panel();
                }
            }
            // the tunnel's chip: its menu, or the kill switch (Ctrl+Shift+F12) that closes every tunnel with no question
            Some("tunel") => {
                if args.get(1).map(String::as_str) == Some("fechar") {
                    tunel_agora::close_all(false);
                } else {
                    tunel_agora::toggle_panel();
                }
            }
            Some("alternar") => {
                let a = self.alternador.borrow().clone();
                match a {
                    Some(a) => a.alternar(false),
                    None if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() => eprintln!("debug alternador: pedido antes de existir"),
                    None => {}
                }
            }
            Some("alternar-voltar") => {
                if let Some(a) = self.alternador.borrow().clone() {
                    a.alternar(true);
                }
            }
            Some("encaixe") => {
                if let Some(e) = self.encaixe.borrow().clone() {
                    e.alternar();
                }
            }
            Some("lateral") => {
                let mode = match args.get(1).map(String::as_str) {
                    Some("controles") => lateral::Modo::Controles,
                    Some("agente") => lateral::Modo::Agente,
                    _ => lateral::Modo::Avisos,
                };
                // "abrir": always open, "fechar": always closed (scripts, tests); otherwise the shortcut toggles
                if args.get(1).map(String::as_str) == Some("fechar") {
                    self.right_bar().close();
                } else if args.get(2).map(String::as_str) == Some("abrir") {
                    self.right_bar().abrir(mode);
                } else {
                    self.right_bar().alternar(mode);
                }
            }
            Some("hoje") => self.today_bar().alternar(),
            // a new conversation in the agents center with this request
            // a request in the agents center's conversation; files with it as --anexo=<path> (before the words)
            Some("conversar") if args.len() > 1 => {
                let anexos: Vec<std::path::PathBuf> = args[1..].iter().filter_map(|a| a.strip_prefix("--anexo=")).map(std::path::PathBuf::from).collect();
                let pedido = args[1..].iter().filter(|a| !a.starts_with("--anexo=")).cloned().collect::<Vec<_>>().join(" ");
                self.abrir_central(Some("conversas"));
                if let Some(c) = self.central.borrow().clone() {
                    c.conversar_com(&pedido, anexos);
                }
            }
            // a request for the agent in the side bar (scripts, other apps, tests)
            // a folder (path or file:// URI, as a .desktop's %U passes it) or where it was
            Some("calculadora") => calculadora::open(self),
            // the side bar's agent stops what it is doing (scripts, tests)
            Some("agente") if args.get(1).map(String::as_str) == Some("parar") => self.right_bar().conversa.stop(true),
            Some("editor") => {
                let p = args.get(1).map(gtk::gio::File::for_commandline_arg).and_then(|f| f.path());
                editor::open(self, p);
            }
            Some("monitor") => monitor::open(self, args.get(1).map(String::as_str)),
            Some("imagens") => image_viewer::open(self, args.get(1).map(std::path::PathBuf::from)),
            Some("musica") => music_player::open(self, args.get(1).map(std::path::PathBuf::from)),
            Some("video") => video_player::open(self, args.get(1).map(std::path::PathBuf::from)),
            Some("documentos") => doc_reader::open(self, args.get(1).map(std::path::PathBuf::from)),
            Some("discos") => disks_app::open(self),
            // the App Center (with a search: `loja TEXT`; on a page: `loja --pagina=modelos`)
            Some("loja") => match args.get(1).and_then(|a| a.strip_prefix("--pagina=")) {
                Some(p) => app_store::open_page(self, p),
                None => app_store::open(self, (args.len() > 1).then(|| args[1..].join(" "))),
            },
            // the models' source card of Settings, alone in a window (session tests: it does not need the flows service);
            // `modelos-fonte alterar` also opens the dialog that changes it
            Some("modelos-fonte") => modelos_fonte::open_card_window(args.get(1).map(String::as_str) == Some("alterar")),
            Some("calendario") => calendar_app::open(self),
            Some("ajuda") => help_app::open(self, (args.len() > 1).then(|| args[1..].join(" "))),
            // a picture of the screen for the agent's "Ver a tela" (as describing the screen: no notice, and the person's
            // clipboard comes back after it)
            Some("tela-para") => {
                if let Some(p) = args.get(1) {
                    descrever::mark_own_capture();
                    if let Err(e) = servicos::niri_acao(serde_json::json!({"ScreenshotScreen": {"write_to_disk": true, "show_pointer": false, "path": p}})) {
                        eprintln!("kalango-shell: tela para o agente: {e}");
                    }
                    glib::timeout_add_local_once(std::time::Duration::from_millis(1500), clipboard::repor_ultimo);
                    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                        eprintln!("debug agente: tela capturada para o agente");
                    }
                }
            }
            Some("terminal") => {
                let p = args.get(1).map(gtk::gio::File::for_commandline_arg).and_then(|f| f.path());
                terminal::open(self, p);
            }
            // an SSH or Mosh host of Kalango Remote in a tab of the Terminal (or its tab to the front)
            // (`--arquivos`: with the SFTP panel open)
            Some("terminal-ssh") => match args.get(1) {
                Some(id) => terminal::open_host(self, id, args.iter().any(|a| a == "--arquivos")),
                None => eprintln!("kalango-shell: terminal-ssh HOST-ID"),
            },
            // the drop-down Terminal (F12; "expandir": Shift+F12)
            Some("terminal-suspenso") => terminal_dropdown::command(self, args),
            Some("arquivos") => {
                let p = args.get(1).map(gtk::gio::File::for_commandline_arg).and_then(|f| f.path());
                self.abrir_arquivos(p.as_deref());
            }
            // the settings, on a section ("rede", "som"…) or a desktop panel's name ("wifi", "display"…)
            Some("configuracoes") => match args.get(1) {
                Some(p) => configuracoes::abrir_painel(p),
                None => self.abrir_configuracoes(None),
            },
            Some("pedir") if args.len() > 1 => self.right_bar().ask_agent(&args[1..].join(" ")),
            // the clipboard history (Super+V); "copiar TEXTO" puts a text on the clipboard (scripts), "--segredo" marks it
            // as a password manager does (it must stay out of the history)
            Some("transferencia") => match args.get(1).map(String::as_str) {
                Some("copiar") => {
                    let secret = args.get(2).map(String::as_str) == Some("--segredo");
                    let text = args[if secret { 3 } else { 2 }..].join(" ");
                    if let Err(e) = clipboard::copy_text(&text, secret) {
                        eprintln!("kalango-shell: copiar: {e}");
                    }
                }
                _ => {
                    if let Some(c) = self.clipboard.borrow().clone() {
                        c.toggle();
                    }
                }
            },
            // voice mode, from the compositor: Super held (iniciar), let go (parar), joined by another key (cancelar)
            Some("voz") => {
                if let Some(v) = self.voice.borrow().clone() {
                    match args.get(1).map(String::as_str) {
                        Some("iniciar") => v.start(),
                        Some("parar") => v.finish(),
                        Some("cancelar") => v.cancel(),
                        _ => {}
                    }
                }
            }
            // the agent's digest of the notifications held for it, now
            Some("avisos") if args.get(1).map(String::as_str) == Some("resumir") => self.notificacoes().deliver_digest(),
            // Super alone, from the compositor (niri patch 0003): tapped, the search; held, voice mode — as the person chose
            // in Settings › Input › Keyboard (either can do nothing)
            Some("super") => {
                let a = configuracoes::ajustes();
                match args.get(1).map(String::as_str) {
                    Some("tocar") if a["super_tocar"].as_bool().unwrap_or(true) => self.comando().alternar(),
                    Some("segurar") if a["super_segurar"].as_bool().unwrap_or(true) => {
                        if let Some(v) = self.voice.borrow().clone() {
                            v.start();
                        }
                    }
                    Some("soltar") | Some("cancelar") => {
                        if let Some(v) = self.voice.borrow().clone() {
                            if args.get(1).map(String::as_str) == Some("soltar") { v.finish() } else { v.cancel() }
                        }
                    }
                    _ => {}
                }
            }
            // the layout of the open windows at their present size (the suite runs it at several screen sizes)
            Some("auditar-layout") => layout_audit::run(),
            // Accessibility: describe the screen (Super+Alt+D), dictation (Super+H)
            Some("descrever-tela") => descrever::descrever_tela(),
            Some("ditar") => descrever::ditar(),
            // the media keys: the change and its on-screen indicator
            Some("osd") => osd::comando(&args[1..]),
            // emoji and symbols (Super+.)
            // the steps' motion, to measure and record it (development session only)
            Some("passos-demo") if std::env::var_os("KALANGO_SHELL_DEV").is_some() => {
                crate::chat_blocks::demo_window(app, args.get(1).map(String::as_str).unwrap_or("rodando"))
            }
            Some("teclado") => osk::toggle(),
            Some("emoji") => {
                if let Some(app) = gtk::gio::Application::default().and_downcast::<gtk::Application>() {
                    emoji::alternar(&app);
                }
            }
            // the first use (designs "Primeiro uso"): after the installation it opens by itself
            Some("primeiro-uso") => {
                if let Some(app) = gtk::gio::Application::default().and_downcast::<gtk::Application>() {
                    primeiro_uso::abrir(&app);
                }
            }
            // what the shell holds in memory (finding leaks): "memoria [N]" logs RSS, heap and the N biggest GObject
            // types (with GOBJECT_DEBUG=instance-count); "memoria aparar" first gives the heap's free pages back
            Some("memoria") => {
                if args.get(1).map(String::as_str) == Some("aparar") {
                    mem_debug::trim();
                }
                let top = args.iter().skip(1).find_map(|a| a.parse().ok()).unwrap_or(25);
                eprintln!("debug memoria: {}", mem_debug::report(top));
            }
            // what an app window holds, to compare a close and reopen with before (sessao/testar-janelas.sh): see
            // window_state
            Some("estado") => window_state::command(&args[1..]),
            Some("sair") => app.quit(),
            Some(o) => eprintln!("kalango-shell: comando desconhecido: {o}"),
            None => {}
        }
    }
}

thread_local! {
    /// the shell, for the free functions that open its windows (the side bar's links, the launcher)
    static SHELL: RefCell<std::rc::Weak<Shell>> = RefCell::default();
}

pub(crate) fn shell_atual() -> Option<Rc<Shell>> {
    SHELL.with(|w| w.borrow().upgrade())
}

fn iniciar(app: &gtk::Application) -> Rc<Shell> {
    let display = gdk::Display::default().expect("o shell precisa de uma sessão Wayland");
    // windows shown again give their focus anew (else their text field never reaches the input method)
    text_focus::watch(app);
    // the AI models: the installer's token to the vault, and what was chosen there to download
    modelos_fonte::start_background();
    // the style sheet in the person's theme (dark, light or by the hour; the accent; the corners)
    tema::iniciar(&display);
    // a lista de protocolos do compositor pode ainda não ter chegado: sincroniza e confere de novo antes de desistir
    for _ in 0..20 {
        if gtk4_layer_shell::is_supported() {
            break;
        }
        display.sync();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if !gtk4_layer_shell::is_supported() {
        eprintln!("kalango-shell: backend {:?}, WAYLAND_DISPLAY={:?}, GDK_BACKEND={:?}", display.backend(), std::env::var("WAYLAND_DISPLAY"), std::env::var("GDK_BACKEND"));
        eprintln!("kalango-shell: o compositor não oferece layer-shell; o shell precisa do niri (ou outro compositor com wlr-layer-shell)");
        std::process::exit(1);
    }
    let todos = apps::todos();
    let fixados = apps::fixados(&todos);
    let shell = Rc::new(Shell { estado: Rc::default(), todos, fixados: RefCell::new(fixados), midia: midia::Players::iniciar(), right_bar: RefCell::default(), today_bar: RefCell::default(), system_listeners: RefCell::default(),
        comando: RefCell::default(), notificacoes: RefCell::default(), energia: RefCell::default(), clipboard: RefCell::default(), voice: RefCell::default(), encaixe: RefCell::default(), alternador: RefCell::default(), grade: RefCell::default(), bloqueio: RefCell::default(), visao: RefCell::default(), area: RefCell::default(), central: RefCell::default(), arquivos: RefCell::default(), configuracoes: RefCell::default(), agendador: RefCell::default(), aviso_pedido: RefCell::default(), usando: RefCell::default(), operacoes: RefCell::default(), dock: RefCell::default(), barra: RefCell::default() });
    SHELL.with(|w| *w.borrow_mut() = Rc::downgrade(&shell));
    let cmd = comando::Comando::new(app);
    cmd.ligar(&shell);
    *shell.comando.borrow_mut() = Some(cmd);
    let energia = energia::Energia::new(app);
    energia.ligar(&shell);
    *shell.energia.borrow_mut() = Some(energia);
    // the clipboard history: every copy from now on, and its panel (Super+V)
    *shell.clipboard.borrow_mut() = Some(clipboard::Clipboard::new(app));
    // voice mode: Super held (the compositor runs "voz iniciar", "voz parar", "voz cancelar")
    let voice = voice_mode::VoiceMode::new(app);
    voice.connect(&shell);
    *shell.voice.borrow_mut() = Some(voice);
    *shell.encaixe.borrow_mut() = Some(encaixe::Encaixe::new(app));
    let bloqueio = bloqueio::Bloqueio::new();
    bloqueio.ligar(&shell);
    *shell.bloqueio.borrow_mut() = Some(bloqueio);
    let visao = visao::Visao::new(app);
    visao.ligar(&shell);
    *shell.visao.borrow_mut() = Some(visao);
    spaces::start(app, &shell);
    // the apps look like KalangoOS (their GTK sheets from the theme's tokens)
    app_theme::start();
    let grade = grade::Grade::new(app);
    grade.ligar(&shell);
    *shell.grade.borrow_mut() = Some(grade);
    let alt = alternador::Alternador::new(app);
    alt.ligar(&shell);
    *shell.alternador.borrow_mut() = Some(alt);

    *shell.notificacoes.borrow_mut() = Some(notificacoes::Notificacoes::iniciar(app));
    // the approval notice: "Ver" shows the conversation where the request is (the side bar's, or the agents center's)
    let aviso = pedido_aviso::AvisoPedido::new(app);
    let sw = Rc::downgrade(&shell);
    aviso.set_abrir(move |conv| {
        let Some(s) = sw.upgrade() else { return };
        if Rc::ptr_eq(&s.right_bar().conversa, conv) {
            s.right_bar().abrir(lateral::Modo::Agente);
        } else {
            s.abrir_central(Some("conversas"));
        }
    });
    // the agent using an app: a banner under the top bar; the approval card goes under it while it shows
    let usando = usando::AgenteUsando::new(app);
    let aw = Rc::downgrade(&aviso);
    usando.ao_mudar(move |visivel| {
        if let Some(a) = aw.upgrade() {
            a.deslocar(if visivel { usando::ALTURA } else { 0 });
        }
    });
    *shell.usando.borrow_mut() = Some(usando);
    // an agent controlling the screen (kalangod's session): the banner, the outline, the agent's pointer
    AGENT_SCREEN.with(|a| *a.borrow_mut() = Some(agent_screen::AgentScreen::new(app, &shell)));
    // long copies live in the island; when one ends, a notice opens the folder it went to
    let ops = operacao::Operacoes::new(app);
    let sw = Rc::downgrade(&shell);
    ops.ao_terminar(move |op| {
        let Some(s) = sw.upgrade() else { return };
        let destino = op.destino.clone();
        let erros = op.progresso.erros.lock().map(|e| e.len()).unwrap_or(0);
        let cancelada = op.progresso.cancelado.load(std::sync::atomic::Ordering::Relaxed);
        let titulo = if cancelada { kalango_i18n::tr("Copy cancelled") } else if op.mover { kalango_i18n::tr("Move finished") } else { kalango_i18n::tr("Copy finished") };
        let corpo = if erros > 0 { kalango_i18n::trf("{n} items could not be copied", &[("n", erros.to_string())]) } else { destino.display().to_string() };
        let sw2 = Rc::downgrade(&s);
        s.notificacoes().notify_local_acoes(kalango_i18n::tr("Files"), titulo, &corpo, &[("mostrar", kalango_i18n::tr("Show in the folder"))], move |_| {
            if let Some(s) = sw2.upgrade() {
                s.abrir_arquivos(Some(&destino));
            }
        });
    });
    *shell.operacoes.borrow_mut() = Some(ops);
    *shell.aviso_pedido.borrow_mut() = Some(aviso);
    let right = lateral::Direita::new(app);
    right.ligar(&shell);
    *shell.right_bar.borrow_mut() = Some(right);
    let rc = shell.right_bar().conversa.clone();
    shell.ligar_pedidos(&rc);
    // the side bar's conversation is on screen only with the bar open, in Agent mode
    let sw = Rc::downgrade(&shell);
    rc.set_visivel(move || sw.upgrade().is_some_and(|s| s.right_bar().mostrando_agente()));
    let today = lateral::Hoje::new(app);
    today.ligar(&shell);
    *shell.today_bar.borrow_mut() = Some(today);
    *shell.area.borrow_mut() = Some(widgets::Area::new(app, &shell));
    let barra = superficies::barra(app, &shell);
    let dock = superficies::dock(app, &shell);
    superficies::seguir_espacos_dos_apps(&shell);
    // the local model: loaded now if the person asked; paused while a game is open (Settings › Agents and AI)
    configuracoes_agentes::session_started();
    {
        let s = Rc::downgrade(&shell);
        shell.estado.ao_abrir_janela(move |_, app_id| {
            let Some(s) = s.upgrade() else { return };
            if let Some(a) = apps::da_janela(app_id, &s.todos) {
                configuracoes_agentes::game_opened(a);
            }
        });
    }
    // the bar and the dock stay usable above the grid and the overview: using them leaves those
    for w in [&barra, &dock] {
        let c = gtk::GestureClick::new();
        c.set_button(0);
        c.set_propagation_phase(gtk::PropagationPhase::Capture);
        let s = Rc::downgrade(&shell);
        c.connect_pressed(move |_, _, _, _| {
            if let Some(s) = s.upgrade() {
                s.fechar_sobreposicoes();
            }
        });
        w.add_controller(c);
    }
    *shell.dock.borrow_mut() = Some(dock);
    *shell.barra.borrow_mut() = Some(barra.clone());
    // the fixed dock is the taskbar: no top bar then
    superficies::refazer_dock();
    // the monitors as the person left them (niri forgets them when it starts), and the bar and the dock on the main one
    configuracoes_tela::reaplicar();
    configuracoes_tela::posicionar_barra_e_dock(&shell);
    papel::ligar();

    // as threads de leitura mandam mensagens; a interface aplica a cada 30 ms (sem trabalho quando não há nada)
    let (tx, rx) = std::sync::mpsc::channel();
    servicos::ouvir_niri(tx.clone());
    servicos::ouvir_kalangod(tx);
    let est = shell.estado.clone();
    let weak_drag = Rc::downgrade(&shell.encaixe.borrow().clone().unwrap());
    glib::timeout_add_local(std::time::Duration::from_millis(30), move || {
        // a drag sends many events: only the last of each batch matters (an end is never skipped)
        let mut drag: Option<(Option<u64>, f64, f64)> = None;
        while let Ok(m) = rx.try_recv() {
            match m {
                servicos::Msg::Niri(v) if v.get("WindowDragChanged").is_some() => {
                    // (the screen's edges and corners rest while a window is dragged and dropped)
                    edges::window_drag();
                    let d = &v["WindowDragChanged"];
                    let ev = (d["id"].as_u64(), d["x"].as_f64().unwrap_or(0.), d["y"].as_f64().unwrap_or(0.));
                    if let (Some((None, ..)), Some(_)) = (drag, ev.0) {
                        // an end followed by a new drag in the same batch: deliver the end first
                        if let Some((id, x, y)) = drag {
                            if let Some(s) = weak_drag.upgrade() {
                                s.drag(id, x, y);
                            }
                        }
                    }
                    drag = Some(ev);
                }
                servicos::Msg::Niri(v) => {
                    est.niri_ok.set(true);
                    est.evento_niri(&v);
                }
                servicos::Msg::NiriCaiu => est.niri_ok.set(false),
                servicos::Msg::Pendentes(n) => {
                    if est.pendentes.get() != n {
                        est.pendentes.set(n);
                        est.avisar();
                    }
                }
            }
        }
        if let (Some((id, x, y)), Some(s)) = (drag, weak_drag.upgrade()) {
            s.drag(id, x, y);
        }
        glib::ControlFlow::Continue
    });
    // first read of the system state so the status icons are real from the start, then every 5 s
    let weak = Rc::downgrade(&shell);
    let refresh = move || {
        let weak = weak.clone();
        lateral::em_segundo_plano(sistema::ler, move |s| {
            if let Some(sh) = weak.upgrade() {
                sh.estado_sistema(&s);
            }
        });
    };
    refresh();
    glib::timeout_add_seconds_local(5, move || {
        refresh();
        glib::ControlFlow::Continue
    });
    // calls: the music lower while one goes on (Settings › Sound)
    let weak = Rc::downgrade(&shell);
    glib::timeout_add_seconds_local(3, move || match weak.upgrade() {
        Some(s) => {
            configuracoes_som::vigiar_chamadas(&s);
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });
    // Settings › Power: the idle times (the screens go dark, the computer suspends), the agents keeping it awake and
    // choosing the power mode; a sample of what the computer draws every minute
    configuracoes_energia::watch_idle();
    // Settings › Sound: the spatial audio left on
    configuracoes_som::resume_spatial();
    let weak = Rc::downgrade(&shell);
    glib::timeout_add_local(std::time::Duration::from_millis(500), move || match weak.upgrade() {
        Some(s) => {
            configuracoes_energia::watch(&s);
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });
    configuracoes_energia::sample_power();
    glib::timeout_add_local(std::time::Duration::from_millis(configuracoes_energia::minute_ms()), || {
        configuracoes_energia::sample_power();
        // critical alerts: the battery at 5%, the disk almost full
        alertas::vigiar();
        glib::ControlFlow::Continue
    });
    // the daily token limit: a notice at 80% and at 100% (once a day each); the night light's schedule
    let weak = Rc::downgrade(&shell);
    let avisado: Rc<Cell<(i64, u8)>> = Rc::default();
    let noturna: Rc<Cell<bool>> = Rc::default();
    configuracoes::vigiar_luz_noturna(&noturna);
    // Privacy › the agents' history: conversations older than the days chosen go
    lateral::em_segundo_plano(configuracoes_privacidade::podar_historico, |n| {
        if n > 0 && configuracoes::depurando() {
            eprintln!("debug privacidade: {n} conversas antigas apagadas");
        }
    });
    // Accessibility kept from before: reduced motion, niri's part, the captions; the screen reader (not in the
    // development and test sessions)
    configuracoes_acessibilidade::aplicar();
    if configuracoes_acessibilidade::ligado("leitor_de_tela") && std::env::var_os("KALANGO_SHELL_DEV").is_none() {
        let _ = std::process::Command::new("orca").arg("--replace").stdin(std::process::Stdio::null()).spawn();
    }
    // NetworkManager asks the shell for Wi-Fi passwords (its secret agent; not in the development session)
    nm_agent::start();
    // and BlueZ asks it how to pair (its pairing agent; not in the development session either)
    bluez_agent::start();
    // drives plugged in: mounted, and their card (open, import the photos, eject)
    removable_media::start();
    // the file chooser of every app (the portal's FileChooser)
    portal::start();
    // the creative apps' automatic updates (Central de Apps; background, applied when the app is closed)
    criativos::agendar();
    // the creative apps' Flatpak permissions made again by kalangod, from its checks: an override from before never outlives the rules
    criativos::refresh_permissions_at_start();
    // polkit asks the shell for the admin password (its authentication agent; not in the development session)
    polkit_agent::start();
    privacy_now::start();
    tunel_agora::start();
    // apps that stop reading the compositor's events: their windows dimmed and "App não está respondendo"
    unresponsive::start(&shell);
    // the divider between windows snapped side by side (design "Janelas lado a lado")
    snap_divider::start();
    // the edges and corners that act (Settings › Appearance › Dock and multitasking) and niri's window rules
    edges::apply();
    window_prefs::write_kdl();
    window_prefs::apply_edge_snap();
    // wide screens: the side bars pinned, if so chosen
    if configuracoes::ajustes()["barras_fixas_largas"].as_bool().unwrap_or(false) && superficies::tamanho_da_saida().is_some_and(|(w, _)| w >= 2560) {
        shell.right_bar().pin_open();
        shell.today_bar().pin_open();
    }
    // what the last shutdown stopped: a notice to continue it
    shutdown_screen::offer_interrupted(&shell);
    // the first session after the installation: the first use (never in the development and test sessions)
    if primeiro_uso::pendente().is_some() && std::env::var_os("KALANGO_SHELL_DEV").is_none() {
        if let Some(app) = gtk::gio::Application::default().and_downcast::<gtk::Application>() {
            primeiro_uso::abrir(&app);
        }
    }
    // Settings › System › Storage: the automatic clean-up of the trash's old files (when the person turned it on)
    if configuracoes::ajustes()["limpeza_automatica"].as_bool().unwrap_or(false) {
        lateral::em_segundo_plano(configuracoes_sistema::clean_old_trash, |n| {
            if n > 0 && configuracoes::depurando() {
                eprintln!("debug sistema: limpeza automática ({n} da lixeira)");
            }
        });
    }
    // Agents › Settings › Advanced › Keep the conversations: the older ones go at the start and once a day
    let limpar_conversas = || {
        let dias = agent_prefs::load().keep_days;
        if dias > 0 {
            lateral::em_segundo_plano(move || agent_prefs::prune_conversations(dias), |_| {});
        }
    };
    limpar_conversas();
    glib::timeout_add_seconds_local(86_400, move || {
        limpar_conversas();
        glib::ControlFlow::Continue
    });
    glib::timeout_add_seconds_local(60, move || match weak.upgrade() {
        Some(s) => {
            central_uso::vigiar_limite(&s, &avisado);
            configuracoes::vigiar_luz_noturna(&noturna);
            configuracoes::vigiar_atualizacao_noturna(&s);
            configuracoes_sistema::watch_space();
            s.notificacoes().watch_digest();
            tema::vigiar();
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });
    // routines: at their time, on a USB device, on entering a space
    *shell.agendador.borrow_mut() = Some(rotinas::Agendador::new(&shell));
    // the drop-down Terminal, ready and hidden when the person asked for it
    terminal_dropdown::at_start(&shell);
    // "learn from what I do": habits from the apps opened, a minute after start and then every 6 hours
    let weak = Rc::downgrade(&shell);
    glib::timeout_add_seconds_local_once(60, move || {
        if let Some(s) = weak.upgrade() {
            central_memoria::observar_habitos(&s);
            let weak = Rc::downgrade(&s);
            glib::timeout_add_seconds_local(6 * 3600, move || match weak.upgrade() {
                Some(s) => {
                    central_memoria::observar_habitos(&s);
                    glib::ControlFlow::Continue
                }
                None => glib::ControlFlow::Break,
            });
        }
    });
    // the system's sounds: the start's or the login's (once a session), the charger's, and the approval screen's
    // decisions (it asks the shell over D-Bus: that screen loads no audio code)
    system_sounds::watch_power_source();
    system_sounds::watch_network();
    system_sounds::watch_update();
    if let Some(app) = gtk::gio::Application::default() {
        system_sounds::export_action(&app);
    }
    system_sounds::at_session_start();
    eprintln!("kalango-shell: pronto ({} apps, {} na dock)", shell.todos.len(), shell.fixados.borrow().len());
    shell
}

/// In development (KALANGO_SHELL_DEV) the system's own apps come from the repository (sessao/dist/share: the agents
/// center's .desktop) and its "kalango-shell …" commands run this same binary. Installed, they are in /usr.
fn dev_paths() {
    if std::env::var_os("KALANGO_SHELL_DEV").is_none() {
        return;
    }
    let share = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../sessao/dist/share"));
    if let Ok(share) = share.canonicalize() {
        let atual = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
        if !atual.split(':').any(|d| std::path::Path::new(d) == share) {
            std::env::set_var("XDG_DATA_DIRS", format!("{}:{atual}", share.display()));
        }
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) {
        let atual = std::env::var("PATH").unwrap_or_default();
        if !atual.split(':').any(|d| std::path::Path::new(d) == dir) {
            std::env::set_var("PATH", format!("{}:{atual}", dir.display()));
        }
    }
}

thread_local! {
    static AGENT_SCREEN: RefCell<Option<Rc<agent_screen::AgentScreen>>> = const { RefCell::new(None) };
}

fn main() -> glib::ExitCode {
    // (a LaTeX compilation's walls: the helper puts them up and becomes the engine, before anything else starts)
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--latex-sandbox") {
        latex_pdf::sandbox_main(&args[2..]);
    }
    dev_paths();
    let app = gtk::Application::builder()
        .application_id(std::env::var("KALANGO_SHELL_APP_ID").unwrap_or_else(|_| "os.kalango.Shell".into()))
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    let shell: Rc<RefCell<Option<Rc<Shell>>>> = Rc::default();
    app.connect_command_line(move |app, cmd| {
        let args: Vec<String> = cmd.arguments().iter().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
        if shell.borrow().is_none() {
            if args.first().map(|a| a == "sair").unwrap_or(false) {
                return 0;
            }
            *shell.borrow_mut() = Some(iniciar(app));
            std::mem::forget(app.hold());
        }
        if let Some(s) = shell.borrow().as_ref() {
            s.executar(app, &args);
        }
        0
    });
    app.run()
}
