//! Command bar (Super+Space), as in the design: "06 · Agentes", frame "Barra de comando · 1 pedir", and
//! "12 · Janelas, busca e sistema", frame "Barra de comando · 2 resultados e resposta".
//!
//! - Empty: an agent prompt (720 px): context chips (the active window, files added with "+ Contexto"),
//!   suggestions for that context, and a footer with the local model and what is shared.
//! - Typing: universal search (1000 px), two columns: results on the left (best result, files from the
//!   index, apps and settings, actions) and, after a pause, the agent's answer on the right (local model,
//!   with its sources and Open / Summarize / Ask more).
//!
//! Keys: ↑ ↓ navigate, ↵ open, Tab actions, Ctrl+↵ ask the agent (side bar conversation), Esc closes.
//! Motion: the backdrop darkens and the card comes down from the bar (0.96 → 1, gentle spring); rows come in
//! with the 24 ms cascade; closing uses the quick spring.

use crate::apps::{self, App};
use crate::conversa::ui_icon;
use crate::indice::{Entry, FileIndex};
use crate::markdown;
use crate::superficies::{caixa, camada, rotulo};
use crate::voz;
use crate::Shell;
use gtk::prelude::*;
use gtk::{gdk, gio, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{n_, tr, trf};
use kalango_movimento::{self as mv, Animavel, Canal};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::{Rc, Weak};
use std::time::Duration;

const PROMPT_W: i32 = 720;
const SEARCH_W: i32 = 1000;
/// design "Barra de comando · 3 resposta que vira interface": the question and its interface, 800 px
const ANSWER_W: i32 = 800;
const MUTED: &str = "#8E8F99";
const ACCENT: &str = "#A5A1E2";

#[derive(Clone)]
enum Item {
    App(App),
    File(PathBuf),
    /// gnome-control-center panel
    Setting(&'static str),
    /// sends the text to the agent (side bar conversation)
    Ask(String),
    /// id of the power menu action
    Power(&'static str),
    OpenFolder(PathBuf),
    /// the mail app, with the file attached
    Email(PathBuf),
}

/// System actions found by name.
const POWER: &[(&str, &str, &str)] = &[
    (n_("Lock screen"), "bloquear", "lock screen bloquear tela travar"),
    (n_("Suspend"), "suspender", "suspend sleep suspender dormir"),
    (n_("Restart"), "reiniciar", "restart reboot reiniciar"),
    (n_("Shut down"), "desligar", "shut down power off desligar"),
    (n_("Log out of the session"), "sair", "log out logout sair sessão encerrar"),
];

/// A settings page (until the KalangoOS Settings app: the desktop's panels).
struct SettingsPage {
    panel: &'static str,
    name: &'static str,
    section: &'static str,
    keywords: &'static str,
}

const fn page(panel: &'static str, name: &'static str, section: &'static str, keywords: &'static str) -> SettingsPage {
    SettingsPage { panel, name, section, keywords }
}

const SETTINGS: &[SettingsPage] = &[
    page("wifi", n_("Wi-Fi"), n_("Network"), "wifi wi-fi rede internet sem fio network wireless"),
    page("network", n_("Network"), n_("Network"), "rede cabo ethernet vpn proxy network wired"),
    page("bluetooth", n_("Bluetooth"), n_("Devices"), "bluetooth fone mouse teclado headphones"),
    page("display", n_("Display"), n_("Devices"), "tela monitor resolução display brilho escala"),
    page("sound", n_("Sound"), n_("Devices"), "som volume áudio audio microfone sound"),
    page("keyboard", n_("Keyboard and shortcuts"), n_("Devices"), "teclado atalhos keyboard shortcuts layout"),
    page("mouse", n_("Mouse and touchpad"), n_("Devices"), "mouse touchpad ponteiro rolagem"),
    page("printers", n_("Printers"), n_("Devices"), "impressoras imprimir printers"),
    page("power", n_("Power"), n_("System"), "energia bateria suspender power battery"),
    page("notifications", n_("Notifications"), n_("System"), "notificações avisos notifications"),
    page("universal-access", n_("Accessibility"), n_("System"), "acessibilidade accessibility zoom leitor"),
    page("system", n_("Storage"), n_("System"), "armazenamento disco espaço storage disk"),
    page("system", n_("About"), n_("System"), "sobre sistema versão about"),
    page("datetime", n_("Date and language"), n_("System"), "data hora idioma fuso language date time"),
    page("privacy", n_("Privacy and security"), n_("Privacy"), "privacidade segurança privacy security bloqueio"),
    page("default-apps", n_("Default apps"), n_("Apps"), "apps padrão default applications navegador"),
];

pub struct Comando {
    janela: gtk::ApplicationWindow,
    fundo: Animavel,
    cartao: Animavel,
    caixa_cartao: gtk::Box,
    topo: gtk::Box,
    entrada: gtk::Entry,
    // prompt state (empty)
    prompt_tools: gtk::Box,
    contexto: gtk::Box,
    divisor: gtk::Box,
    sugestoes: gtk::Box,
    rodape_pedido: gtk::Box,
    compartilhando: gtk::Label,
    // search state
    search_tools: gtk::Box,
    corpo: gtk::Box,
    resultados: gtk::Box,
    resposta: gtk::Box,
    rodape_busca: gtk::Box,
    /// an interface answer took the card: the hints under it (ask more, pin, close)
    dicas_ui: gtk::Box,
    modo_ui: Cell<bool>,
    filtro: gtk::MenuButton,
    filtro_rotulo: gtk::Label,
    filtro_sel: Cell<u32>,
    itens: RefCell<Vec<(Item, gtk::Widget)>>,
    sel: Cell<usize>,
    pub aberta: Cell<bool>,
    apps_only: Cell<bool>,
    shell: RefCell<Weak<Shell>>,
    indice: FileIndex,
    /// (app name, window title) of the active window, if the person keeps it in the context
    janela_ctx: RefCell<Option<(String, String)>>,
    arquivos_ctx: RefCell<Vec<PathBuf>>,
    answer_gen: Cell<u64>,
    answer_proc: RefCell<Option<Child>>,
    best: RefCell<Option<Item>>,
    /// files that mention the typed phrase: (the query they answer, hits with their line)
    content_hits: RefCell<(String, Vec<(Entry, usize)>)>,
    content_gen: Cell<u64>,
    /// the files shown for the query, best first (given to the agent as context)
    shown_files: RefCell<Vec<Entry>>,
    gravando: RefCell<Option<voz::Recording>>,
    mic: gtk::Button,
}

fn secao(t: &str) -> gtk::Label {
    let l = rotulo(t, "cmd-secao");
    l.set_halign(Align::Start);
    l
}

fn key(t: &str) -> gtk::Label {
    let l = rotulo(t, "tecla");
    l.set_valign(Align::Center);
    l
}

/// The filter options ("Tudo" in the design), in order: all, files, apps, actions.
fn filter_names() -> [&'static str; 4] {
    [tr("All"), tr("Files"), tr("Apps"), tr("Actions")]
}

fn human_size(b: u64) -> String {
    match b {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{} KB", b >> 10),
        b => format!("{b} B"),
    }
}

fn short_dir(p: &Path) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let d = p.parent().map(|d| d.display().to_string()).unwrap_or_default();
    d.strip_prefix(&format!("{home}/")).map(str::to_string).unwrap_or_else(|| if d == home { "~".into() } else { d })
}

fn when(modified: i64) -> String {
    let now = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(modified);
    match (now - modified).max(0) {
        s if s < 86400 => tr("today").to_string(),
        s if s < 2 * 86400 => tr("yesterday").to_string(),
        _ => glib::DateTime::from_unix_local(modified).ok().and_then(|d| d.format("%d/%m").ok()).map(|s| s.to_string()).unwrap_or_default(),
    }
}

fn kind_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_else(|| tr("File").to_string())
}

fn is_image(p: &Path) -> bool {
    matches!(p.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref(), Some("png" | "jpg" | "jpeg" | "webp" | "gif" | "svg"))
}

fn file_glyph(p: &Path) -> &'static str {
    match p.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
        Some("sh" | "py" | "rs" | "js" | "ts" | "toml" | "json" | "c" | "h") => "terminal",
        _ => "doc",
    }
}

impl Comando {
    pub fn new(app: &gtk::Application) -> Rc<Comando> {
        let janela = camada(app, "kalango-barra-de-comando", Layer::Overlay, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
        janela.set_exclusive_zone(-1);
        let raiz = gtk::Overlay::new();
        let escuro = caixa(Orientation::Vertical, 0, "fundo");
        escuro.set_hexpand(true);
        escuro.set_vexpand(true);
        let fundo = Animavel::new(&escuro);
        raiz.set_child(Some(&fundo));

        let caixa_cartao = caixa(Orientation::Vertical, 0, "cmd");
        caixa_cartao.set_size_request(PROMPT_W, -1);
        // input row: spark, field, then mic + send (prompt) or filter + Esc (search)
        let topo = caixa(Orientation::Horizontal, 12, "cmd-entrada");
        topo.append(&ui_icon("spark", ACCENT, 16));
        let entrada = gtk::Entry::new();
        entrada.set_hexpand(true);
        entrada.add_css_class("cmd-campo");
        entrada.set_placeholder_text(Some(tr("Ask or tell KalangoOS…")));
        topo.append(&entrada);
        let prompt_tools = caixa(Orientation::Horizontal, 12, "");
        let mic = gtk::Button::new();
        mic.set_child(Some(&ui_icon("microfone", MUTED, 16)));
        mic.add_css_class("cmd-icone");
        mic.set_tooltip_text(Some(tr("Dictate (the local voice model writes it here)")));
        prompt_tools.append(&mic);
        let enviar = gtk::Button::new();
        enviar.set_child(Some(&ui_icon("enviar", "#1C1D24", 16)));
        enviar.add_css_class("botao-enviar");
        enviar.set_tooltip_text(Some(tr("Ask the agent (Ctrl+Enter)")));
        prompt_tools.append(&enviar);
        topo.append(&prompt_tools);
        let search_tools = caixa(Orientation::Horizontal, 12, "");
        // filter: a chevron and the current choice, with the options in a menu
        let filtro = gtk::MenuButton::new();
        filtro.set_always_show_arrow(false);
        filtro.set_valign(Align::Center);
        let fb = caixa(Orientation::Horizontal, 5, "");
        fb.append(&ui_icon("seta-baixo", MUTED, 16));
        let filtro_rotulo = rotulo(tr("All"), "cmd-filtro-texto");
        fb.append(&filtro_rotulo);
        filtro.set_child(Some(&fb));
        filtro.add_css_class("cmd-filtro");
        search_tools.append(&filtro);
        search_tools.append(&key("Esc"));
        topo.append(&search_tools);
        caixa_cartao.append(&topo);

        // prompt state
        let contexto = caixa(Orientation::Horizontal, 6, "cmd-contexto");
        caixa_cartao.append(&contexto);
        let divisor = caixa(Orientation::Horizontal, 0, "cmd-divisor");
        caixa_cartao.append(&divisor);
        let sugestoes = caixa(Orientation::Vertical, 2, "cmd-sugestoes");
        caixa_cartao.append(&sugestoes);
        let rodape_pedido = caixa(Orientation::Horizontal, 10, "cmd-rodape-pedido");
        let modelo = caixa(Orientation::Horizontal, 6, "cmd-modelo");
        modelo.append(&ui_icon("cerebro", ACCENT, 14));
        modelo.append(&rotulo(&format!("{} · {}", local_model_label(), tr("local")), "cmd-modelo-nome"));
        rodape_pedido.append(&modelo);
        let esp = gtk::Box::new(Orientation::Horizontal, 0);
        esp.set_hexpand(true);
        rodape_pedido.append(&esp);
        rodape_pedido.append(&ui_icon("escudo", "#8ED2C3", 14));
        let compartilhando = rotulo("", "cmd-rodape-texto");
        rodape_pedido.append(&compartilhando);
        rodape_pedido.append(&rotulo("Esc", "cmd-rodape-esc"));
        caixa_cartao.append(&rodape_pedido);

        // search state: results | answer
        let corpo = caixa(Orientation::Horizontal, 0, "cmd-corpo");
        let rolagem = gtk::ScrolledWindow::new();
        rolagem.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        rolagem.set_propagate_natural_height(true);
        rolagem.set_max_content_height(428);
        rolagem.set_min_content_height(200);
        rolagem.set_size_request(500, -1);
        let resultados = caixa(Orientation::Vertical, 4, "cmd-resultados");
        rolagem.set_child(Some(&resultados));
        corpo.append(&rolagem);
        corpo.append(&caixa(Orientation::Vertical, 0, "cmd-divisor-v"));
        let resposta = caixa(Orientation::Vertical, 12, "cmd-resposta");
        resposta.set_hexpand(true);
        corpo.append(&resposta);
        caixa_cartao.append(&corpo);
        let rodape_busca = caixa(Orientation::Horizontal, 14, "cmd-rodape-busca");
        for (keys, label) in [(vec!["↑", "↓"], tr("navigate")), (vec!["↵"], tr("open")), (vec!["Tab"], tr("actions")), (vec!["Ctrl", "↵"], tr("ask the agent"))] {
            let g = caixa(Orientation::Horizontal, 4, "");
            for k in keys {
                g.append(&key(k));
            }
            g.append(&rotulo(label, "cmd-rodape-texto"));
            rodape_busca.append(&g);
        }
        caixa_cartao.append(&rodape_busca);
        // under the card, when an answer became an interface
        let dicas_ui = caixa(Orientation::Horizontal, 14, "cmd-dicas-ui");
        dicas_ui.set_halign(Align::Center);
        for (keys, label) in [(vec!["↵"], tr("ask more")), (vec!["Ctrl", "P"], tr("pin to the desktop")), (vec!["Esc"], tr("close"))] {
            let g = caixa(Orientation::Horizontal, 4, "");
            for k in keys {
                g.append(&key(k));
            }
            g.append(&rotulo(label, "cmd-rodape-texto"));
            dicas_ui.append(&g);
        }
        dicas_ui.set_visible(false);
        let pilha = caixa(Orientation::Vertical, 10, "");
        pilha.append(&caixa_cartao);
        pilha.append(&dicas_ui);

        let cartao = Animavel::new(&pilha);
        cartao.origem(0.5, 0.0);
        cartao.set_halign(Align::Center);
        cartao.set_valign(Align::Start);
        cartao.set_margin_top(120);
        raiz.add_overlay(&cartao);
        janela.set_child(Some(&raiz));

        let c = Rc::new(Comando {
            janela, fundo, cartao, caixa_cartao, topo, entrada, prompt_tools, contexto, divisor, sugestoes, rodape_pedido, compartilhando,
            search_tools, corpo, resultados, resposta, rodape_busca, dicas_ui, modo_ui: Cell::new(false), filtro, filtro_rotulo, filtro_sel: Cell::new(0), itens: RefCell::default(), sel: Cell::new(0),
            aberta: Cell::new(false), apps_only: Cell::new(false), shell: RefCell::default(), indice: FileIndex::start(),
            janela_ctx: RefCell::default(), arquivos_ctx: RefCell::default(), answer_gen: Cell::new(0), answer_proc: RefCell::default(),
            best: RefCell::default(), content_hits: RefCell::default(), content_gen: Cell::new(0), shown_files: RefCell::default(),
            gravando: RefCell::default(), mic,
        });
        {
            let w = Rc::downgrade(&c);
            c.entrada.connect_changed(move |e| {
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug busca: {:?}", e.text());
                }
                if let Some(c) = w.upgrade() {
                    c.refresh(false);
                }
            });
            let w = Rc::downgrade(&c);
            c.entrada.connect_activate(move |_| {
                if let Some(c) = w.upgrade() {
                    c.activate();
                }
            });
            let menu = gtk::Popover::new();
            menu.add_css_class("menu");
            let lista = caixa(Orientation::Vertical, 0, "");
            for (i, name) in filter_names().into_iter().enumerate() {
                let b = gtk::Button::with_label(name);
                b.add_css_class("flat");
                b.add_css_class("cmd-filtro-opcao");
                let w = Rc::downgrade(&c);
                let m = menu.downgrade();
                b.connect_clicked(move |_| {
                    if let Some(m) = m.upgrade() {
                        m.popdown();
                    }
                    if let Some(c) = w.upgrade() {
                        c.set_filter(i as u32);
                        c.entrada.grab_focus();
                    }
                });
                lista.append(&b);
            }
            menu.set_child(Some(&lista));
            c.filtro.set_popover(Some(&menu));
            let w = Rc::downgrade(&c);
            enviar.connect_clicked(move |_| {
                if let Some(c) = w.upgrade() {
                    c.ask_typed();
                }
            });
            let w = Rc::downgrade(&c);
            c.mic.connect_clicked(move |_| {
                if let Some(c) = w.upgrade() {
                    c.toggle_dictation();
                }
            });
        }
        let teclas = gtk::EventControllerKey::new();
        teclas.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let w = Rc::downgrade(&c);
            teclas.connect_key_pressed(move |_, k, _, m| {
                let Some(c) = w.upgrade() else { return glib::Propagation::Proceed };
                let ctrl = m.contains(gdk::ModifierType::CONTROL_MASK);
                match k {
                    gdk::Key::Escape => c.fechar(),
                    gdk::Key::Down => c.mover(1),
                    gdk::Key::Up => c.mover(-1),
                    gdk::Key::Return | gdk::Key::KP_Enter if ctrl || c.modo_ui.get() => c.ask_typed(),
                    // the interface's "+ Fixar como widget"
                    gdk::Key::p | gdk::Key::P if ctrl && c.modo_ui.get() => {
                        if let Some(b) = crate::conversa::descendant_with_class(&c.resposta, "cmd-ui-fixar").and_downcast::<gtk::Button>() {
                            b.emit_clicked();
                        }
                    }
                    // Tab: to the actions (in the empty prompt there are only suggestions)
                    gdk::Key::Tab | gdk::Key::ISO_Left_Tab => c.to_actions(),
                    _ => return glib::Propagation::Proceed,
                }
                glib::Propagation::Stop
            });
        }
        c.janela.add_controller(teclas);
        // clicking outside the card closes
        let clique = gtk::GestureClick::new();
        {
            let w = Rc::downgrade(&c);
            clique.connect_pressed(move |_, _, x, y| {
                let Some(c) = w.upgrade() else { return };
                let dentro = c.cartao.compute_bounds(&c.janela).map(|b| b.contains_point(&gtk::graphene::Point::new(x as f32, y as f32))).unwrap_or(true);
                if !dentro {
                    c.fechar();
                }
            });
        }
        c.fundo.add_controller(clique);
        c
    }

    /// The file index (the desktop's "Agora" card looks for notes about an event in it).
    pub fn indice(&self) -> FileIndex {
        self.indice.clone()
    }

    pub fn ligar(&self, shell: &Rc<Shell>) {
        *self.shell.borrow_mut() = Rc::downgrade(shell);
    }

    fn shell(&self) -> Option<Rc<Shell>> {
        self.shell.borrow().upgrade()
    }

    fn todos(&self) -> Vec<App> {
        self.shell().map(|s| s.todos.clone()).unwrap_or_default()
    }

    // ------------------------------------------------------------------------------------------------ states

    fn is_prompt(&self) -> bool {
        self.entrada.text().trim().is_empty() && !self.apps_only.get()
    }

    /// Rebuilds the visible state for the current text.
    fn refresh(self: &Rc<Self>, cascade: bool) {
        let prompt = self.is_prompt();
        self.caixa_cartao.set_size_request(if prompt { PROMPT_W } else { SEARCH_W }, -1);
        if prompt { self.caixa_cartao.remove_css_class("busca") } else { self.caixa_cartao.add_css_class("busca") }
        // design: the prompt's text starts 10 px after the spark, the search's 12 px
        self.topo.set_spacing(if prompt { 10 } else { 12 });
        // design: the prompt sits at y = 190, the search at y = 120
        self.cartao.set_margin_top(if prompt { 190 } else { 120 });
        let prompt_parts: [gtk::Widget; 5] = [self.prompt_tools.clone().upcast(), self.contexto.clone().upcast(), self.divisor.clone().upcast(), self.sugestoes.clone().upcast(), self.rodape_pedido.clone().upcast()];
        for w in prompt_parts {
            w.set_visible(prompt);
        }
        let search_parts: [gtk::Widget; 3] = [self.search_tools.clone().upcast(), self.corpo.clone().upcast(), self.rodape_busca.clone().upcast()];
        for w in search_parts {
            w.set_visible(!prompt);
        }
        if prompt {
            self.fill_prompt(cascade);
            self.cancel_answer();
        } else {
            self.fill_results(cascade);
            self.schedule_content();
            self.schedule_answer();
        }
    }

    fn fill_prompt(self: &Rc<Self>, cascade: bool) {
        // context chips: the active window and the files added
        while let Some(w) = self.contexto.first_child() {
            self.contexto.remove(&w);
        }
        if let Some((app, _title)) = self.janela_ctx.borrow().clone() {
            let me = Rc::downgrade(self);
            self.contexto.append(&chip("pasta", &trf("Window: {app}", &[("app", app)]), move || {
                if let Some(c) = me.upgrade() {
                    *c.janela_ctx.borrow_mut() = None;
                    c.refresh(false);
                }
            }));
        }
        let files: Vec<PathBuf> = self.arquivos_ctx.borrow().clone();
        for (i, f) in files.iter().enumerate() {
            let me = Rc::downgrade(self);
            let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            self.contexto.append(&chip("doc", &name, move || {
                if let Some(c) = me.upgrade() {
                    c.arquivos_ctx.borrow_mut().remove(i);
                    c.refresh(false);
                }
            }));
        }
        let add = gtk::Button::new();
        let ab = caixa(Orientation::Horizontal, 6, "");
        ab.append(&ui_icon("mais", MUTED, 14));
        ab.append(&rotulo(tr("Context"), "cmd-contexto-mais"));
        add.set_child(Some(&ab));
        add.add_css_class("cmd-contexto-botao");
        let me = Rc::downgrade(self);
        add.connect_clicked(move |_| {
            if let Some(c) = me.upgrade() {
                c.add_file_context();
            }
        });
        self.contexto.append(&add);
        let window = self.janela_ctx.borrow().is_some();
        self.compartilhando.set_text(match (window, files.is_empty()) {
            (true, _) => tr("Sharing: active window"),
            (false, false) => tr("Sharing: the added files"),
            (false, true) => tr("Nothing shared"),
        });

        // suggestions for this context
        while let Some(w) = self.sugestoes.first_child() {
            self.sugestoes.remove(&w);
        }
        let app = self.janela_ctx.borrow().as_ref().map(|(a, _)| a.to_lowercase()).unwrap_or_default();
        let mut sug: Vec<(&str, String)> = Vec::new();
        if app.contains("arquivos") || app.contains("files") || app.contains("nautilus") {
            sug.push(("pasta", tr("Organize Downloads by type and date").to_string()));
        }
        if app.contains("terminal") || app.contains("console") {
            sug.push(("terminal", tr("Explain the last error in the terminal").to_string()));
        }
        if let Some(f) = files.first() {
            sug.push(("doc", trf("Summarize {file}", &[("file", f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())])));
        }
        for (g, t) in [("relogio", tr("What did the agents do today?")), ("olho", tr("What is using the most memory?")), ("pasta", tr("How much free space do I have?"))] {
            if sug.len() < 4 {
                sug.push((g, t.to_string()));
            }
        }
        let mut itens = Vec::new();
        let mut anims = Vec::new();
        for (g, t) in sug {
            let row = caixa(Orientation::Horizontal, 10, "cmd-sugestao");
            row.append(&ui_icon(g, MUTED, 16));
            let l = rotulo(&t, "cmd-sugestao-texto");
            l.set_xalign(0.0);
            l.set_hexpand(true);
            row.append(&l);
            row.append(&rotulo("↵", "cmd-enter"));
            let a = Animavel::new(&row);
            self.sugestoes.append(&a);
            anims.push(a.clone());
            let click = gtk::GestureClick::new();
            let me = Rc::downgrade(self);
            let t2 = t.clone();
            click.connect_released(move |_, _, _, _| {
                if let Some(c) = me.upgrade() {
                    c.run(Item::Ask(t2.clone()));
                }
            });
            a.add_controller(click);
            itens.push((Item::Ask(t), a.upcast::<gtk::Widget>()));
        }
        if cascade {
            mv::cascata(&anims, 6.0);
        }
        *self.itens.borrow_mut() = itens;
        self.select(0);
    }

    fn fill_results(self: &Rc<Self>, cascade: bool) {
        while let Some(w) = self.resultados.first_child() {
            self.resultados.remove(&w);
        }
        let q = self.entrada.text().trim().to_string();
        let qq = q.to_lowercase();
        let (want_files, want_apps, want_actions) = match self.filtro_sel.get() {
            1 => (true, false, false),
            2 => (false, true, false),
            3 => (false, false, true),
            _ => (true, true, true),
        };
        let todos = self.todos();
        let apps: Vec<App> = if self.apps_only.get() && qq.is_empty() { todos.clone() } else if want_apps { apps::buscar(&todos, &qq).into_iter().cloned().collect() } else { vec![] };
        let files: Vec<Entry> = if want_files && !qq.is_empty() { self.indice.search(&qq, 6) } else { vec![] };
        let settings: Vec<&SettingsPage> = if want_apps && !qq.is_empty() {
            SETTINGS.iter().filter(|p| tr(p.name).to_lowercase().contains(&qq) || p.keywords.contains(&qq)).take(3).collect()
        } else { vec![] };
        let power: Vec<&(&str, &str, &str)> = if want_actions && !qq.is_empty() {
            POWER.iter().filter(|(n, _, k)| tr(n).to_lowercase().contains(&qq) || k.contains(&qq)).collect()
        } else { vec![] };

        // files that mention the phrase (found in the background for this same query)
        let mentions: Vec<(Entry, usize)> = {
            let ch = self.content_hits.borrow();
            if want_files && ch.0 == q { ch.1.clone() } else { vec![] }
        };
        // best result: the top file when its name starts like the query, else the first app, else the first file
        let first_word = crate::indice::fold(qq.split_whitespace().next().unwrap_or(""));
        let best: Option<Item> = if self.apps_only.get() { None } else {
            files.first().filter(|f| crate::indice::fold(&f.path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default()).starts_with(&first_word))
                .map(|f| Item::File(f.path.clone()))
                .or_else(|| apps.first().map(|a| Item::App(a.clone())))
                .or_else(|| files.first().map(|f| Item::File(f.path.clone())))
                .or_else(|| mentions.first().map(|(e, _)| Item::File(e.path.clone())))
        };
        *self.best.borrow_mut() = best.clone();

        let mut blocks: Vec<(gtk::Widget, Option<Item>)> = Vec::new();
        if let Some(b) = &best {
            blocks.push((secao(tr("BEST RESULT")).upcast(), None));
            blocks.push((self.best_row(b, &files).upcast(), Some(b.clone())));
        }
        let is_best = |p: &Path| matches!(&best, Some(Item::File(b)) if b == p);
        let other_files: Vec<&Entry> = files.iter().filter(|f| !is_best(&f.path)).take(4).collect();
        let other_mentions: Vec<&(Entry, usize)> = mentions.iter().filter(|(e, _)| !is_best(&e.path) && !files.iter().any(|f| f.path == e.path)).take(2).collect();
        // what the agent gets as context: the files shown, best first
        let mut shown: Vec<Entry> = Vec::new();
        if let Some(Item::File(p)) = &best {
            if let Some(e) = files.iter().chain(mentions.iter().map(|(e, _)| e)).find(|e| e.path == *p) {
                shown.push(e.clone());
            }
        }
        shown.extend(other_files.iter().map(|e| (*e).clone()));
        shown.extend(other_mentions.iter().map(|(e, _)| e.clone()));
        *self.shown_files.borrow_mut() = shown;
        if !other_files.is_empty() || !other_mentions.is_empty() {
            blocks.push((secao(tr("FILES")).upcast(), None));
            for f in other_files {
                let sub = format!("{} · {}", short_dir(&f.path), when(f.modified));
                let name = f.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                blocks.push((row(ui_icon(file_glyph(&f.path), MUTED, 16).upcast(), &name, Some(&sub)).upcast(), Some(Item::File(f.path.clone()))));
            }
            for (f, line) in other_mentions {
                let sub = trf("mentions “{phrase}” · line {line}", &[("phrase", q.clone()), ("line", line.to_string())]);
                let name = f.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                blocks.push((row(ui_icon(file_glyph(&f.path), MUTED, 16).upcast(), &name, Some(&sub)).upcast(), Some(Item::File(f.path.clone()))));
            }
        }
        let other_apps: Vec<&App> = apps.iter().filter(|a| !matches!(&best, Some(Item::App(b)) if b.id == a.id)).take(if self.apps_only.get() { 200 } else { 4 }).collect();
        if !other_apps.is_empty() || !settings.is_empty() {
            blocks.push((secao(if self.apps_only.get() { tr("ALL APPS") } else { tr("APPS AND SETTINGS") }).upcast(), None));
            for a in other_apps {
                blocks.push((row(apps::imagem(a, 22).upcast(), &a.nome, None).upcast(), Some(Item::App(a.clone()))));
            }
            for p in settings {
                let sub = format!("{} › {}", tr("Settings"), tr(p.section));
                blocks.push((row(ui_icon("ajustes", MUTED, 16).upcast(), tr(p.name), Some(&sub)).upcast(), Some(Item::Setting(p.panel))));
            }
        }
        if want_actions && !qq.is_empty() {
            blocks.push((secao(tr("ACTIONS")).upcast(), None));
            if let Some(Item::File(p)) = &best {
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                blocks.push((action_row("spark", &trf("Summarize {file}", &[("file", name)])).upcast(), Some(Item::Ask(trf("Summarize the file {path}", &[("path", p.display().to_string())])))));
                blocks.push((action_row("pasta", tr("Open the folder")).upcast(), Some(Item::OpenFolder(p.parent().map(Path::to_path_buf).unwrap_or_default()))));
                if has_mail_helper() {
                    blocks.push((action_row("enviar", tr("Send by email…")).upcast(), Some(Item::Email(p.clone()))));
                }
            }
            blocks.push((action_row("spark", &trf("Ask the agent: “{query}”", &[("query", q.clone())])).upcast(), Some(Item::Ask(q.clone()))));
            for (n, id, _) in power {
                blocks.push((action_row("energia", tr(n)).upcast(), Some(Item::Power(id))));
            }
        }
        if blocks.iter().all(|(_, i)| i.is_none()) {
            let v = rotulo(&trf("Nothing found for “{query}”.", &[("query", q.clone())]), "vazio");
            v.set_halign(Align::Start);
            blocks.push((v.upcast(), None));
        }
        let mut itens: Vec<(Item, gtk::Widget)> = Vec::new();
        let mut anims = Vec::new();
        for (w, item) in blocks {
            let a = Animavel::new(&w);
            self.resultados.append(&a);
            anims.push(a.clone());
            if let Some(i) = item {
                let click = gtk::GestureClick::new();
                let me = Rc::downgrade(self);
                let i2 = i.clone();
                click.connect_released(move |_, _, _, _| {
                    if let Some(c) = me.upgrade() {
                        c.run(i2.clone());
                    }
                });
                a.add_controller(click);
                itens.push((i, a.upcast()));
            }
        }
        mv::cascata(&anims, if cascade { 6.0 } else { 3.0 });
        *self.itens.borrow_mut() = itens;
        self.select(0);
        self.resposta.set_visible(!self.apps_only.get());
    }

    /// The best result: a larger row with a preview (page, image or app icon), name and details.
    fn best_row(&self, item: &Item, files: &[Entry]) -> gtk::Box {
        let r = caixa(Orientation::Horizontal, 12, "cmd-melhor");
        let (name, meta): (String, String) = match item {
            Item::File(p) => {
                let preview: gtk::Widget = if is_image(p) {
                    let pic = gtk::Picture::for_filename(p);
                    pic.set_content_fit(gtk::ContentFit::Cover);
                    pic.set_size_request(44, 56);
                    pic.add_css_class("cmd-previa-imagem");
                    pic.upcast()
                } else {
                    // a page with lines, as in the design
                    let page = caixa(Orientation::Vertical, 5, "cmd-previa-pagina");
                    page.set_size_request(44, 56);
                    for w in [32, 24, 32, 24] {
                        let l = caixa(Orientation::Horizontal, 0, "cmd-previa-linha");
                        l.set_size_request(w, 3);
                        l.set_halign(Align::Start);
                        page.append(&l);
                    }
                    page.upcast()
                };
                r.append(&preview);
                let meta = match files.iter().find(|f| f.path == *p) {
                    Some(e) => format!("{} · {} · {} · {}", kind_of(p), human_size(e.size), short_dir(p), when(e.modified)),
                    None => short_dir(p),
                };
                (p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), meta)
            }
            Item::App(a) => {
                r.append(&apps::imagem(a, 40));
                (a.nome.clone(), a.info.generic_name().map(|g| g.to_string()).unwrap_or_else(|| tr("App").to_string()))
            }
            _ => (String::new(), String::new()),
        };
        let col = caixa(Orientation::Vertical, 2, "");
        col.set_valign(Align::Center);
        col.set_hexpand(true);
        let n = rotulo(&name, "cmd-melhor-nome");
        n.set_xalign(0.0);
        n.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        col.append(&n);
        let m = rotulo(&meta, "cmd-melhor-meta");
        m.set_xalign(0.0);
        m.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        col.append(&m);
        r.append(&col);
        let k = key("↵");
        k.set_valign(Align::Center);
        r.append(&k);
        r
    }

    // ------------------------------------------------------------------------------------------------ answer

    fn cancel_answer(&self) {
        self.answer_gen.set(self.answer_gen.get() + 1);
        if let Some(mut c) = self.answer_proc.borrow_mut().take() {
            let _ = c.kill();
            std::thread::spawn(move || {
                let _ = c.wait();
            });
        }
    }

    fn answer_header(&self) {
        while let Some(w) = self.resposta.first_child() {
            self.resposta.remove(&w);
        }
        let lr = caixa(Orientation::Horizontal, 6, "");
        lr.append(&ui_icon("spark", ACCENT, 16));
        lr.append(&rotulo(tr("AGENT ANSWER"), "cmd-secao-r"));
        self.resposta.append(&lr);
    }

    fn answer_placeholder(&self, text: &str) {
        self.answer_header();
        let l = rotulo(text, "cmd-resposta-espera");
        l.set_xalign(0.0);
        l.set_wrap(true);
        // a centred card takes its natural width: long texts must not widen it past the design's 1000 px
        l.set_max_width_chars(56);
        self.resposta.append(&l);
    }

    /// After a pause in typing, the local agent answers the query (with its tools) on the right.
    fn schedule_answer(self: &Rc<Self>) {
        self.cancel_answer();
        // (an interface answer took the whole card: the results come back for another question)
        self.answer_mode(false);
        let q = self.entrada.text().trim().to_string();
        if q.chars().count() < 4 || self.apps_only.get() {
            self.answer_placeholder(tr("Keep typing: the agent answers here when you pause."));
            return;
        }
        self.answer_placeholder(tr("Thinking…"));
        let gen = self.answer_gen.get();
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(900), move || {
            let Some(c) = w.upgrade() else { return };
            if c.answer_gen.get() != gen || !c.aberta.get() {
                return;
            }
            // what fills the disk, what uses the memory: answered with an interface measured here (design "Barra de
            // comando · 3 resposta que vira interface")
            if crate::disk_answer::is_question(&q) {
                c.answer_placeholder(tr("Measuring the disk…"));
                let w = Rc::downgrade(&c);
                crate::lateral::em_segundo_plano(crate::disk_answer::measure, move |r| {
                    let Some(c) = w.upgrade() else { return };
                    let (sh, close) = (c.shell.borrow().clone(), c.closer());
                    c.show_interface(gen, crate::disk_answer::widget(&r, sh, close));
                });
                return;
            }
            if crate::mem_answer::is_question(&q) {
                c.answer_placeholder(tr("Measuring the memory…"));
                let w = Rc::downgrade(&c);
                crate::lateral::em_segundo_plano(crate::mem_answer::measure, move |r| {
                    let Some(c) = w.upgrade() else { return };
                    let (sh, close) = (c.shell.borrow().clone(), c.closer());
                    c.show_interface(gen, crate::mem_answer::widget(&r, sh, close));
                });
                return;
            }
            c.run_answer(gen, &q);
        });
    }

    /// The bar closing (an interface's action that opens an app).
    fn closer(self: &Rc<Self>) -> Rc<dyn Fn()> {
        let cw = Rc::downgrade(self);
        Rc::new(move || if let Some(c) = cw.upgrade() { c.fechar() })
    }

    /// An interface answer: the whole card (the results step aside until another question).
    fn show_interface(self: &Rc<Self>, gen: u64, w: gtk::Widget) {
        if self.answer_gen.get() != gen || !self.aberta.get() {
            return;
        }
        while let Some(x) = self.resposta.first_child() {
            self.resposta.remove(&x);
        }
        self.answer_mode(true);
        // (born from the request field above it: the soft spring)
        let a = Animavel::new(&w);
        a.origem(0.5, 0.0);
        self.resposta.append(&a);
        a.entrar((0.0, -12.0), 0.97);
    }

    /// The card as the question and its interface only (no results, no filter; its own hints under it) — or back.
    fn answer_mode(&self, on: bool) {
        if self.modo_ui.replace(on) == on {
            return;
        }
        let mut f = self.corpo.first_child();
        while let Some(x) = f {
            f = x.next_sibling();
            if x != *self.resposta.upcast_ref::<gtk::Widget>() {
                x.set_visible(!on);
            }
        }
        self.filtro.set_visible(!on);
        self.rodape_busca.set_visible(!on && !self.is_prompt());
        self.dicas_ui.set_visible(on);
        if on {
            self.resposta.add_css_class("cmd-ui-modo");
            self.caixa_cartao.set_size_request(ANSWER_W, -1);
        } else {
            self.resposta.remove_css_class("cmd-ui-modo");
            self.caixa_cartao.set_size_request(if self.is_prompt() { PROMPT_W } else { SEARCH_W }, -1);
        }
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug comando: modo resposta {}", if on { "ligado" } else { "desligado" });
        }
    }

    /// After a short pause, text files that mention the typed phrase are looked for in the background; they join
    /// the file results when found (if the query is still the same).
    fn schedule_content(self: &Rc<Self>) {
        let gen = self.content_gen.get() + 1;
        self.content_gen.set(gen);
        let q = self.entrada.text().trim().to_string();
        if q.chars().count() < 4 || self.apps_only.get() || self.content_hits.borrow().0 == q {
            return;
        }
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(200), move || {
            let Some(c) = w.upgrade() else { return };
            if c.content_gen.get() != gen || !c.aberta.get() {
                return;
            }
            let idx = c.indice.clone();
            let phrase = q.clone();
            let w = Rc::downgrade(&c);
            crate::lateral::em_segundo_plano(move || idx.search_content(&phrase, 3), move |hits| {
                let Some(c) = w.upgrade() else { return };
                if c.content_gen.get() != gen || c.entrada.text().trim() != q {
                    return;
                }
                let changed = !hits.is_empty();
                *c.content_hits.borrow_mut() = (q, hits);
                if changed {
                    let sel = c.sel.get();
                    c.fill_results(false);
                    c.select(sel.min(c.itens.borrow().len().saturating_sub(1)));
                }
            });
        });
    }

    fn run_answer(self: &Rc<Self>, gen: u64, q: &str) {
        let files: Vec<String> = self.shown_files.borrow().iter().take(4)
            .map(|e| format!("- {} ({}, {}, {})", e.path.display(), kind_of(&e.path), human_size(e.size), when(e.modified))).collect();
        let mut request = format!("{q}\n\n{}", tr("This was typed in the computer's search. Answer in up to 3 short bullet points (•), without a title and without naming tools. Cite PDF pages as (p. N)."));
        if !files.is_empty() {
            request.push_str(&format!("\n{}\n{}", tr("Files the search found (if one of them is what is being looked for, read it and answer about its content):"), files.join("\n")));
        }
        let child = Command::new(voz::agent_binary()).arg("--json").arg(&request)
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
        let Ok(mut child) = child else {
            self.answer_placeholder(tr("The agent is not available."));
            return;
        };
        let stdout = child.stdout.take();
        *self.answer_proc.borrow_mut() = Some(child);
        let (tx, rx) = std::sync::mpsc::channel::<Value>();
        if let Some(out) = stdout {
            std::thread::spawn(move || {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    if let Ok(v) = serde_json::from_str::<Value>(&line) {
                        if tx.send(v).is_err() {
                            return;
                        }
                    }
                }
            });
        }
        let w = Rc::downgrade(self);
        let title = {
            let mut ch = q.chars();
            ch.next().map(|f| f.to_uppercase().collect::<String>() + ch.as_str()).unwrap_or_default()
        };
        let sources: Rc<RefCell<Vec<String>>> = Rc::default();
        let used_web = Rc::new(Cell::new(false));
        glib::timeout_add_local(Duration::from_millis(50), move || {
            let Some(c) = w.upgrade() else { return glib::ControlFlow::Break };
            if c.answer_gen.get() != gen {
                return glib::ControlFlow::Break;
            }
            loop {
                match rx.try_recv() {
                    Ok(v) => match v["tipo"].as_str() {
                        Some("ferramenta") => {
                            // sources are what the agent read (files and web pages), not folder listings
                            let name = v["nome"].as_str().unwrap_or("").replace("__", ".");
                            let a = &v["argumentos"];
                            let src = match name.as_str() {
                                "arquivos.ler" => a["caminho"].as_str(),
                                "web.ler" => a["url"].as_str(),
                                "web.buscar" => a["consulta"].as_str(),
                                _ => None,
                            };
                            if let Some(p) = src {
                                if !sources.borrow().iter().any(|s| s == p) {
                                    sources.borrow_mut().push(p.to_string());
                                }
                            }
                            if name.starts_with("web.") {
                                used_web.set(true);
                            }
                        }
                        Some("resposta") => c.show_answer(&title, v["texto"].as_str().unwrap_or(""), &sources.borrow(), used_web.get()),
                        Some("parado") => c.answer_placeholder(&trf("Stopped: {reason}", &[("reason", v["motivo"].as_str().unwrap_or("").to_string())])),
                        Some("erro") => c.answer_placeholder(v["texto"].as_str().unwrap_or("")),
                        _ => {}
                    },
                    Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(_) => return glib::ControlFlow::Break,
                }
            }
        });
    }

    fn show_answer(self: &Rc<Self>, title: &str, text: &str, sources: &[String], used_web: bool) {
        self.answer_header();
        let t = rotulo(title, "cmd-resposta-titulo");
        t.set_xalign(0.0);
        t.set_wrap(true);
        t.set_max_width_chars(40);
        self.resposta.append(&t);
        let md = markdown::render(text);
        md.add_css_class("cmd-resposta-texto");
        cap_label_width(md.upcast_ref(), 56);
        self.resposta.append(&md);
        if !sources.is_empty() {
            // content-sized chips in a row (at most 4, so they fit the 460 px column)
            let flow = caixa(Orientation::Horizontal, 6, "");
            flow.set_halign(Align::Start);
            // a PDF read with pages cited in the answer: one chip per page ("relatorio-out.pdf · p. 2", "p. 7")
            // the pages belong to the best result's PDF when the agent read it, else to the first PDF it read
            let pages = cited_pages(text);
            let best_path = match &*self.best.borrow() {
                Some(Item::File(p)) => Some(p.display().to_string()),
                _ => None,
            };
            let is_pdf = |s: &str| s.to_lowercase().ends_with(".pdf");
            let paged = best_path.filter(|b| is_pdf(b) && sources.contains(b)).or_else(|| sources.iter().find(|s| is_pdf(s)).cloned());
            let mut chips: Vec<(String, String)> = Vec::new();
            let name_of = |s: &str| Path::new(s).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| s.to_string());
            match (&paged, pages.is_empty()) {
                (Some(pdf), false) => {
                    for (i, p) in pages.iter().take(3).enumerate() {
                        chips.push((pdf.clone(), if i == 0 { format!("{} · p. {p}", name_of(pdf)) } else { format!("p. {p}") }));
                    }
                    chips.extend(sources.iter().filter(|s| *s != pdf).map(|s| (s.clone(), name_of(s))));
                }
                _ => chips.extend(sources.iter().map(|s| (s.clone(), name_of(s)))),
            }
            for (s, label) in chips.iter().take(4) {
                let b = gtk::Button::new();
                let bb = caixa(Orientation::Horizontal, 5, "");
                bb.append(&ui_icon("doc", MUTED, 14));
                bb.append(&rotulo(label, "cmd-fonte-nome"));
                b.set_child(Some(&bb));
                b.add_css_class("cmd-fonte");
                b.set_tooltip_text(Some(s));
                let path = s.clone();
                let me = Rc::downgrade(self);
                b.connect_clicked(move |_| {
                    if let Some(c) = me.upgrade() {
                        c.fechar();
                        open_uri(&path);
                    }
                });
                flow.append(&b);
            }
            self.resposta.append(&flow);
        }
        let actions = caixa(Orientation::Horizontal, 8, "");
        let best = self.best.borrow().clone();
        if let Some(b) = best {
            let open = gtk::Button::with_label(tr("Open"));
            open.add_css_class("cmd-botao-principal");
            let me = Rc::downgrade(self);
            open.connect_clicked(move |_| {
                if let Some(c) = me.upgrade() {
                    c.run(b.clone());
                }
            });
            actions.append(&open);
        }
        let more = gtk::Button::with_label(tr("Summarize in 5 points"));
        more.add_css_class("cmd-botao");
        let me = Rc::downgrade(self);
        let q = self.entrada.text().to_string();
        let q2 = q.clone();
        more.connect_clicked(move |_| {
            if let Some(c) = me.upgrade() {
                c.run(Item::Ask(trf("Summarize in 5 points: {topic}", &[("topic", q2.clone())])));
            }
        });
        actions.append(&more);
        let ask = gtk::Button::with_label(tr("Ask more"));
        ask.add_css_class("cmd-botao-fantasma");
        let me = Rc::downgrade(self);
        ask.connect_clicked(move |_| {
            if let Some(c) = me.upgrade() {
                c.run(Item::Ask(q.clone()));
            }
        });
        actions.append(&ask);
        self.resposta.append(&actions);
        let pr = caixa(Orientation::Horizontal, 6, "");
        pr.append(&ui_icon("escudo", "#8ED2C3", 14));
        pr.append(&rotulo(if used_web { tr("Answered by the local model · it read web pages for this") } else { tr("Answered by the local model · nothing left this computer") }, "cmd-rodape-texto"));
        self.resposta.append(&pr);
    }

    fn set_filter(self: &Rc<Self>, i: u32) {
        self.filtro_sel.set(i);
        self.filtro_rotulo.set_text(filter_names()[i as usize]);
        if self.aberta.get() {
            self.refresh(false);
        }
    }

    // ------------------------------------------------------------------------------------------------ selection

    fn select(&self, i: usize) {
        let itens = self.itens.borrow();
        for (k, (_, w)) in itens.iter().enumerate() {
            if k == i { w.add_css_class("selecionado") } else { w.remove_css_class("selecionado") }
        }
        self.sel.set(i);
    }

    fn mover(&self, d: i32) {
        let n = self.itens.borrow().len() as i32;
        if n == 0 {
            return;
        }
        self.select(((self.sel.get() as i32 + d).rem_euclid(n)) as usize);
    }

    fn to_actions(&self) {
        let idx = self.itens.borrow().iter().position(|(i, _)| matches!(i, Item::Ask(_) | Item::Power(_) | Item::OpenFolder(_) | Item::Email(_)));
        if let Some(i) = idx {
            self.select(i);
        }
    }

    fn activate(self: &Rc<Self>) {
        let item = self.itens.borrow().get(self.sel.get()).map(|(i, _)| i.clone());
        if let Some(item) = item {
            self.run(item);
        } else if !self.entrada.text().trim().is_empty() {
            self.ask_typed();
        }
    }

    /// Ctrl+Enter / send: what was typed goes to the agent, with the context.
    fn ask_typed(self: &Rc<Self>) {
        let q = self.entrada.text().trim().to_string();
        if q.is_empty() {
            if let Some(s) = self.shell() {
                self.fechar();
                s.right_bar().abrir(crate::lateral::Modo::Agente);
            }
            return;
        }
        self.run(Item::Ask(q));
    }

    fn with_context(&self, request: &str) -> String {
        let mut ctx = Vec::new();
        if let Some((app, title)) = self.janela_ctx.borrow().as_ref() {
            ctx.push(trf("active window: {app} — {title}", &[("app", app.clone()), ("title", title.clone())]));
        }
        for f in self.arquivos_ctx.borrow().iter() {
            ctx.push(trf("file: {path}", &[("path", f.display().to_string())]));
        }
        if ctx.is_empty() { request.to_string() } else { format!("{request}\n\n({}: {})", tr("Context"), ctx.join("; ")) }
    }

    fn run(self: &Rc<Self>, item: Item) {
        let request = if let Item::Ask(t) = &item { Some(self.with_context(t)) } else { None };
        self.fechar();
        match item {
            Item::App(a) => {
                if let Err(e) = apps::abrir(&a) {
                    eprintln!("kalango-shell: abrir {}: {e}", a.nome);
                }
            }
            Item::File(p) | Item::OpenFolder(p) => open_uri(&p.display().to_string()),
            Item::Email(p) => {
                let _ = Command::new("xdg-email").arg("--attach").arg(&p).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
            }
            Item::Setting(panel) => {
                crate::configuracoes::abrir_painel(panel);
            }
            Item::Power(id) => {
                if let Some(s) = self.shell() {
                    s.energia().executar(id);
                }
            }
            Item::Ask(_) => {
                if let (Some(s), Some(r)) = (self.shell(), request) {
                    s.right_bar().ask_agent(&r);
                }
            }
        }
    }

    /// A file for the bar's context, from the system's chooser.
    fn add_file_context(self: &Rc<Self>) {
        let me = Rc::downgrade(self);
        crate::arquivos::seletor::abrir_arquivo("", tr("Add a file to the context"), Some(tr("Add")), Vec::new(), None, move |p| {
            if let Some(c) = me.upgrade() {
                c.arquivos_ctx.borrow_mut().push(p);
                c.refresh(false);
            }
        });
    }

    fn toggle_dictation(self: &Rc<Self>) {
        if let Some(rec) = self.gravando.borrow_mut().take() {
            self.mic.remove_css_class("gravando");
            self.entrada.set_placeholder_text(Some(tr("Transcribing…")));
            let w = Rc::downgrade(self);
            crate::lateral::em_segundo_plano(move || voz::finish(rec), move |r: Result<String, String>| {
                let Some(c) = w.upgrade() else { return };
                c.entrada.set_placeholder_text(Some(tr("Ask or tell KalangoOS…")));
                if let Ok(t) = r {
                    if !t.trim().is_empty() {
                        c.entrada.set_text(t.trim());
                        c.entrada.set_position(-1);
                    }
                }
            });
            return;
        }
        if let Ok(rec) = voz::start("comando") {
            *self.gravando.borrow_mut() = Some(rec);
            self.mic.add_css_class("gravando");
            self.entrada.set_placeholder_text(Some(tr("Listening… click the microphone to stop")));
        }
    }

    // ------------------------------------------------------------------------------------------------ open/close

    /// The active window (niri) goes into the context by default, as in the design ("Janela: Arquivos").
    fn read_active_window(&self) {
        let v = crate::servicos::niri_pedido(json!("FocusedWindow")).ok().and_then(|v| v.get("FocusedWindow").cloned()).unwrap_or(Value::Null);
        let app_id = v["app_id"].as_str().unwrap_or("");
        let title = v["title"].as_str().unwrap_or("").to_string();
        if app_id.is_empty() {
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug contexto: nenhuma janela em foco ({v})");
            }
            *self.janela_ctx.borrow_mut() = None;
            return;
        }
        let todos = self.todos();
        let name = apps::da_janela(app_id, &todos).map(|a| a.nome.clone()).unwrap_or_else(|| app_id.to_string());
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug contexto: janela {name}");
        }
        *self.janela_ctx.borrow_mut() = Some((name, title));
    }

    pub fn abrir(self: &Rc<Self>, texto: &str) {
        self.aberta.set(true);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug busca: aberta");
        }
        self.read_active_window();
        self.arquivos_ctx.borrow_mut().clear();
        self.filtro_sel.set(if self.apps_only.get() { 2 } else { 0 });
        self.filtro_rotulo.set_text(filter_names()[self.filtro_sel.get() as usize]);
        self.entrada.set_text(texto);
        self.refresh(true);
        self.janela.present();
        // exclusive keyboard after mapping: the compositor re-evaluates the focus
        let w = self.janela.downgrade();
        glib::timeout_add_local_once(Duration::from_millis(40), move || {
            if let Some(j) = w.upgrade() {
                crate::superficies::take_keyboard(&j);
            }
        });
        self.fundo.ir(Canal::Opacidade, 1.0, mv::SUAVE);
        self.cartao.entrar((0.0, -8.0), 0.96);
        self.entrada.grab_focus();
    }

    pub fn abrir_apps(self: &Rc<Self>) {
        self.apps_only.set(true);
        self.abrir("");
    }

    pub fn fechar(&self) {
        if !self.aberta.replace(false) {
            return;
        }
        self.apps_only.set(false);
        self.cancel_answer();
        self.answer_mode(false);
        if let Some(rec) = self.gravando.borrow_mut().take() {
            self.mic.remove_css_class("gravando");
            std::thread::spawn(move || {
                let _ = voz::finish(rec);
            });
        }
        self.janela.set_keyboard_mode(KeyboardMode::None);
        self.fundo.ir(Canal::Opacidade, 0.0, mv::RAPIDA);
        let j = self.janela.downgrade();
        let ainda = self.shell.borrow().clone();
        self.cartao.sair((0.0, -8.0), 0.98, move || {
            // reopened during the exit? then it stays
            let reaberta = ainda.upgrade().map(|s| s.comando().aberta.get()).unwrap_or(false);
            if let (Some(j), false) = (j.upgrade(), reaberta) {
                j.set_visible(false);
            }
        });
    }

    pub fn alternar(self: &Rc<Self>) {
        if self.aberta.get() {
            self.fechar();
        } else {
            self.abrir("");
        }
    }
}

/// Every label under `w` wraps within `chars` (natural width capped).
fn cap_label_width(w: &gtk::Widget, chars: i32) {
    if let Some(l) = w.downcast_ref::<gtk::Label>() {
        l.set_max_width_chars(chars);
    }
    let mut c = w.first_child();
    while let Some(x) = c {
        cap_label_width(&x, chars);
        c = x.next_sibling();
    }
}

/// Page numbers cited in an answer as "p. N" or "[p. N]", in order, without repeats.
fn cited_pages(text: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("p.") {
        let after = rest[i + 2..].trim_start();
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        let boundary = i == 0 || !rest[..i].chars().last().is_some_and(|c| c.is_alphanumeric());
        if boundary && !digits.is_empty() {
            if let Ok(n) = digits.parse::<u32>() {
                if !out.contains(&n) {
                    out.push(n);
                }
            }
        }
        rest = &rest[i + 2..];
    }
    out
}

fn has_mail_helper() -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("xdg-email").is_file()))
}

fn local_model_label() -> String {
    let dir = std::env::var_os("KALANGO_MODELOS").map(PathBuf::from).unwrap_or_else(|| {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
            .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")).join("kalango/modelos")
    });
    if dir.join("LFM3-1.7B-Q4_K_M.gguf").is_file() { "LFM3 1.7B".into() } else { "LFM3".into() }
}

fn open_uri(path_or_url: &str) {
    let uri = if path_or_url.contains("://") { path_or_url.to_string() } else { gio::File::for_path(path_or_url).uri().to_string() };
    let ctx = gdk::Display::default().map(|d| d.app_launch_context());
    let _ = crate::apps::open_uri(&uri, ctx.as_ref().map(|c| c.upcast_ref()));
}

/// A context chip (design "Chip de contexto"): icon, text and an × that removes it.
fn chip(glyph: &str, text: &str, on_remove: impl Fn() + 'static) -> gtk::Box {
    let c = caixa(Orientation::Horizontal, 6, "cmd-chip");
    c.append(&ui_icon(glyph, MUTED, 14));
    let l = rotulo(text, "cmd-chip-texto");
    l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    l.set_max_width_chars(28);
    c.append(&l);
    let x = gtk::Button::new();
    x.set_child(Some(&ui_icon("fechar", "#7A7B88", 12)));
    x.add_css_class("cmd-chip-fechar");
    x.connect_clicked(move |_| on_remove());
    c.append(&x);
    c
}

fn row(icon: gtk::Widget, name: &str, sub: Option<&str>) -> gtk::Box {
    let b = caixa(Orientation::Horizontal, 12, "cmd-linha");
    icon.set_valign(Align::Center);
    b.append(&icon);
    let col = caixa(Orientation::Vertical, 1, "");
    col.set_valign(Align::Center);
    col.set_hexpand(true);
    let n = rotulo(name, "cmd-linha-titulo");
    n.set_halign(Align::Start);
    n.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    col.append(&n);
    if let Some(s) = sub.filter(|s| !s.is_empty()) {
        let l = rotulo(s, "cmd-linha-sub");
        l.set_halign(Align::Start);
        l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        col.append(&l);
    }
    b.append(&col);
    b.append(&rotulo("↵", "cmd-enter"));
    b
}

fn action_row(glyph: &str, title: &str) -> gtk::Box {
    let icon: gtk::Widget = ui_icon(glyph, if glyph == "spark" { ACCENT } else { MUTED }, 16).upcast();
    let b = row(icon, title, None);
    b.add_css_class("cmd-acao");
    b
}

#[cfg(test)]
mod tests {
    #[test]
    fn pages_cited_in_an_answer() {
        assert_eq!(super::cited_pages("• Receita subiu 8% (p. 2).\n• Pendências [p. 2] e backup (p.7); ver app. 3"), vec![2, 7]);
        assert!(super::cited_pages("sem páginas").is_empty());
    }
}
