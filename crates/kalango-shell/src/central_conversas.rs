//! Agents center › Conversas (designs "Chat · nova conversa" and "Chat · conversa"): the agent as a chat app.
//!
//! On the left, the rail opens into a side bar: the pages, "Nova conversa", the search, the kept conversations
//! (pinned, today, yesterday, earlier) and the person with today's tokens. In the middle, a new conversation (a
//! greeting, the composer and things to start from) or a kept one — the same conversation as the side bar's
//! (`conversa::Conversa`: one engine per conversation, its history on disk, so it goes on where it stopped). On the
//! right, its context: what it read and wrote, the tools this conversation may use (on top of the permissions), the
//! memories it saw, its tokens.

use crate::agent_picker::{self, Picked, Picker};
use crate::conversa::{self, ui_icon, Conversa};
use crate::resize_handle::{self, panel_width, HIT};
use crate::widths::{self, PanelKind};
use crate::superficies::{caixa, estrela, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{gio, glib, Align, Orientation};
use kalango_i18n::{n_, tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// The tool groups a conversation can turn off: (key, name, glyph, the groups it stands for).
const GRUPOS: &[(&str, &str, &str, &[&str])] = &[
    ("web", n_("Web"), "globo", &["dominio:web."]),
    ("arquivos", n_("Files"), "pasta", &["dominio:arquivos."]),
    ("terminal", n_("Terminal"), "terminal", &["dominio:tarefas.", "dominio:sistema.executar"]),
    ("tela", n_("See the screen"), "monitor", &["dominio:tela."]),
];

/// The new conversation's widest measure (its composer, centred); a conversation's column follows
/// `widths::reading_column` instead.
const LEITURA: i32 = 860;
/// What the conversation keeps at least beside the side panels (they give way first).
const CONVERSA_MINIMA: i32 = crate::widths::CONVERSATION_MIN;
/// The side bar's width by default (22 % of the window, within these) and by hand.
const LISTA_PADRAO: (i32, i32) = (240, 360);
const LISTA_LIMITES: (i32, i32) = (220, 420);

/// A side panel's width the person kept ("contexto" or "lista"), from the shell's settings.
fn guardada(qual: &str) -> Option<i32> {
    crate::configuracoes::ajustes()["central_paineis"][qual].as_i64().map(|w| w as i32)
}

fn agora() -> i64 {
    glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0)
}

fn inicio_do_dia() -> i64 {
    glib::DateTime::now_local().ok().and_then(|d| glib::DateTime::from_local(d.year(), d.month(), d.day_of_month(), 0, 0, 0.0).ok()).map(|d| d.to_unix()).unwrap_or(0)
}

fn quando(t: i64) -> String {
    let n = agora();
    if n - t < 120 {
        return tr("now").to_string();
    }
    if n - t < 3600 {
        return trf("{n} min ago", &[("n", ((n - t) / 60).to_string())]);
    }
    let d = glib::DateTime::from_unix_local(t).ok();
    if t >= inicio_do_dia() - 86_400 {
        d.and_then(|d| d.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default()
    } else {
        d.and_then(|d| d.format("%-d %b").ok()).map(|s| s.to_lowercase()).unwrap_or_default()
    }
}

fn mil(n: u64) -> String {
    if n >= 1000 { format!("{}k", (n + 500) / 1000) } else { n.to_string() }
}

/// The window controls (minimize, maximize, close) for this page's own header rows.
fn controles() -> gtk::Box {
    let b = caixa(Orientation::Horizontal, 8, "");
    b.set_valign(Align::Center);
    b.connect_realize(|b| {
        if let Some(w) = b.root().and_downcast::<gtk::Window>() {
            crate::context_menu::title_bar(&w);
        }
    });
    for (glifo, acao) in [("minimizar", 0), ("maximizar", 1), ("fechar", 2)] {
        let x = gtk::Button::new();
        x.add_css_class("central-controle");
        x.set_child(Some(&ui_icon(glifo, "#8E8F99", 16)));
        x.connect_clicked(move |x| {
            let Some(j) = x.root().and_downcast::<gtk::Window>() else { return };
            match acao {
                0 => j.minimize(),
                1 => if j.is_maximized() { j.unmaximize() } else { j.maximize() },
                _ => j.close(),
            }
        });
        b.append(&x);
    }
    b
}

type Ir = Box<dyn Fn(&str)>;
/// Which kept conversations go in a section of the side bar.
type Filtro = Box<dyn Fn(&Value) -> bool>;
/// (the rail is shown, its button that opens the side bar is shown)
type Recolher = Box<dyn Fn(bool, bool)>;

/// The window's width band (design "Chat · fluido"): from 1400 px both side bars are open; from 1000 the side bar
/// is the rail and the context folds into its tabs; below that one column, the side bars open as sheets over it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Faixa {
    Larga,
    Media,
    Estreita,
}

impl Faixa {
    pub fn de(largura: i32) -> Faixa {
        if largura >= 1400 {
            Faixa::Larga
        } else if largura >= 1000 {
            Faixa::Media
        } else {
            Faixa::Estreita
        }
    }
}

pub struct Conversas {
    /// the middle and the context on the right (the page)
    pub raiz: gtk::Box,
    /// the side bar the rail opens into (the agents center places it)
    pub lateral: gtk::Box,
    shell: Weak<Shell>,
    ir: RefCell<Option<Ir>>,
    recolher: RefCell<Option<Recolher>>,
    pub recolhida: Cell<bool>,
    /// the window's band, and the page is the one showing
    faixa: Cell<Faixa>,
    largura: Cell<i32>,
    na_pagina: Cell<bool>,
    /// the context is open (the band's default, then the person's choice)
    painel_aberto: Cell<bool>,
    arrumando: Cell<bool>,
    /// the window's overlay (where the side bars go as sheets in a narrow window) and the dimming under them
    folhas: RefCell<Option<gtk::Overlay>>,
    veu: gtk::Box,
    /// where the side bar and the context live when they are not sheets: (the box, the sibling before)
    casa_lateral: RefCell<Option<(gtk::Box, Option<gtk::Widget>)>>,
    casa_painel: RefCell<Option<(gtk::Box, Option<gtk::Widget>)>>,
    /// the context folded into its tabs (medium window)
    faixa_abas: gtk::Box,
    /// its tabs (glyph, button): the one of the context's open tab is marked
    faixa_botoes: Vec<(&'static str, gtk::Button)>,
    controles_painel: gtk::Box,
    // the narrow window's header: the side bar's button, the agent's star and the line under the title
    abrir_lista_b: gtk::Button,
    faisca: gtk::Image,
    subtitulo: gtk::Label,
    /// the names on the new conversation's tool chips (only the icons when the middle is narrow)
    nomes_chips: Vec<gtk::Label>,
    /// the page showed already (the first time follows "Start on")
    ja_mostrou: Cell<bool>,
    // the side bar
    nova_b: gtk::Button,
    busca: gtk::Entry,
    lista: gtk::Box,
    rodape_tokens: gtk::Label,
    // the middle
    titulo: gtk::Label,
    chip_agente: gtk::Label,
    fixar: gtk::Button,
    apagar: gtk::Button,
    painel_b: gtk::ToggleButton,
    vista: gtk::Stack,
    conv: Rc<Conversa>,
    controles_meio: gtk::Box,
    saudacao: gtk::Label,
    nova_texto: gtk::TextView,
    nova_grupos: Vec<(&'static str, gtk::ToggleButton)>,
    sugestoes: gtk::FlowBox,
    // the context, on the right
    painel: gtk::Box,
    abas: Vec<gtk::ToggleButton>,
    painel_stack: gtk::Stack,
    contexto: gtk::Box,
    ferramentas: gtk::Box,
    memoria: gtk::Box,
    arquivos: gtk::Box,
    tokens_v: gtk::Label,
    tokens_barra: gtk::LevelBar,
    modelo_v: gtk::Label,
    marca: Cell<(u64, u64)>,
    /// the Canvas tab (the latest diagram: its source, a change to ask, how it looks, export)
    canvas_box: gtk::Box,
    /// the canvas with live HTML (design "Chat · canvas com HTML ao vivo"): a page the agent wrote
    html: Rc<crate::html_canvas::HtmlCanvas>,
    /// the agent's browser (the Browser tab) and the context's usual width
    navegador: Rc<crate::agent_browser::AgentBrowser>,
    painel_base: i32,
    /// the widths the person gave the context and the side bar by dragging their edges (None: the default share of
    /// the window), and those edges
    largura_contexto: Cell<Option<i32>>,
    largura_lista: Cell<Option<i32>>,
    /// the same for the Canvas and the Browser, each its own (a page needs more room than the context's rows)
    canvas_width: Cell<Option<i32>>,
    browser_width: Cell<Option<i32>>,
    /// the canvas expanded over most of the window, and whether the side bar was closed before (it gives way)
    expanded: Cell<bool>,
    list_closed_before: Cell<bool>,
    expand_button: gtk::Button,
    alca_contexto: RefCell<Option<gtk::Widget>>,
    pub alca_lista: RefCell<Option<gtk::Widget>>,
    /// the Changes tab and the repository the conversation's files are in (with the files)
    alteracoes_box: gtk::Box,
    /// the commands the agent ran in this conversation, as terminal blocks (design "Chat · código" › Terminal)
    terminal_box: gtk::Box,
    /// the conversation's tokens, model and cost (not under Changes or Terminal: the commit sits at the bottom there)
    uso: gtk::Box,
    repo: RefCell<Option<(std::path::PathBuf, Vec<String>)>>,
    /// the commit made from the tab (its hash: "Desfazer" undoes exactly it)
    commit_feito: RefCell<Option<String>>,
    /// the file whose preview the Files tab shows (the first one until another is chosen)
    previa: RefCell<Option<String>>,
    preenchendo: Cell<bool>,
    // ---- the person's own agents (design "Chat · escolher agente")
    /// the new conversation's agent chips (the system's, the recent ones, "Todos os agentes") and its choice
    start_chips: gtk::Box,
    start_agent: RefCell<Option<Value>>,
    start_picker: RefCell<Option<Rc<Picker>>>,
    /// the new conversation's fields, when its agent asks for them
    nova_form: Rc<agent_picker::InputForm>,
    nova_ph: gtk::Label,
    /// the header's agent chip: its icon and name
    chip_agente_box: gtk::Box,
    chip_tile: gtk::Box,
    /// runs a flow (its id) and shows it live (the agents center)
    ao_executar: RefCell<Option<Ir>>,
    nova_erro: gtk::Label,
}

impl Conversas {
    pub fn new(shell: &Rc<Shell>, largura_janela: i32) -> Rc<Conversas> {
        let compacta = largura_janela < 1480;
        // ---- the side bar
        let lateral = caixa(Orientation::Vertical, 0, "conversas-lateral");
        lateral.set_size_request(if compacta { 230 } else { 270 }, -1);
        lateral.set_hexpand(false);
        let lc = caixa(Orientation::Horizontal, 10, "");
        lc.append(&crate::apps::imagem_kalango("agentes", 22));
        let lt = rotulo(tr("Agents"), "conversas-lateral-titulo");
        lt.set_hexpand(true);
        lt.set_xalign(0.0);
        lc.append(&lt);
        let recolher_b = gtk::Button::new();
        recolher_b.add_css_class("central-controle");
        recolher_b.set_child(Some(&ui_icon("layout", "#8E8F99", 16)));
        recolher_b.set_tooltip_text(Some(tr("Collapse the side bar")));
        lc.append(&recolher_b);
        let lch = gtk::WindowHandle::new();
        lch.set_child(Some(&lc));
        lateral.append(&lch);
        let nova = gtk::Button::new();
        nova.add_css_class("conversas-nova");
        let nc = caixa(Orientation::Horizontal, 8, "");
        nc.set_halign(Align::Center);
        nc.append(&ui_icon("mais", "#E2E2E6", 14));
        nc.append(&rotulo(tr("New conversation"), ""));
        nova.set_child(Some(&nc));
        nova.set_margin_top(14);
        lateral.append(&nova);
        let bc = caixa(Orientation::Horizontal, 7, "conversas-busca");
        bc.set_margin_top(10);
        bc.append(&estrela(12, "#A5A1E2"));
        let busca = gtk::Entry::new();
        busca.add_css_class("central-busca-campo");
        busca.set_placeholder_text(Some(tr("Search conversations…")));
        busca.set_hexpand(true);
        busca.set_width_chars(4);
        bc.append(&busca);
        lateral.append(&bc);
        let nav = caixa(Orientation::Vertical, 2, "");
        nav.set_margin_top(12);
        let mut nav_botoes = Vec::new();
        for (id, nome, glifo, _) in crate::central::PAGINAS {
            let b = gtk::Button::new();
            b.add_css_class("conversas-nav");
            if *id == "conversas" {
                b.add_css_class("ativo");
            }
            let c = caixa(Orientation::Horizontal, 12, "");
            c.append(&if *glifo == "spark" { estrela(13, "#C9CAD1").upcast::<gtk::Widget>() } else { ui_icon(glifo, "#C9CAD1", 16).upcast() });
            c.append(&rotulo(tr(nome), ""));
            b.set_child(Some(&c));
            nav.append(&b);
            nav_botoes.push((*id, b));
        }
        lateral.append(&nav);
        let lista = caixa(Orientation::Vertical, 2, "");
        let lrol = gtk::ScrolledWindow::new();
        lrol.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        lrol.set_vexpand(true);
        lrol.set_child(Some(&lista));
        lrol.set_margin_top(8);
        lateral.append(&lrol);
        let rodape = caixa(Orientation::Horizontal, 10, "conversas-pessoa");
        let nome_pessoa = glib::real_name().to_string_lossy().split(',').next().unwrap_or("").trim().to_string();
        let nome_pessoa = if nome_pessoa.is_empty() || nome_pessoa == "Unknown" { glib::user_name().to_string_lossy().into_owned() } else { nome_pessoa };
        let av = caixa(Orientation::Horizontal, 0, "conversas-avatar");
        av.set_size_request(30, 30);
        av.set_hexpand(false);
        av.set_valign(Align::Center);
        let ini = rotulo(&nome_pessoa.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default(), "conversas-avatar-letra");
        ini.set_hexpand(true);
        av.append(&ini);
        rodape.append(&av);
        let rcol = caixa(Orientation::Vertical, 1, "");
        rcol.set_hexpand(true);
        let rn = rotulo(&nome_pessoa, "central-limite-nome");
        rn.set_xalign(0.0);
        rn.set_ellipsize(gtk::pango::EllipsizeMode::End);
        rn.set_max_width_chars(1);
        rcol.append(&rn);
        let rodape_tokens = rotulo("", "central-agente-sub");
        rodape_tokens.set_xalign(0.0);
        rodape_tokens.set_ellipsize(gtk::pango::EllipsizeMode::End);
        rodape_tokens.set_max_width_chars(1);
        rcol.append(&rodape_tokens);
        rodape.append(&rcol);
        let engr = gtk::Button::new();
        engr.add_css_class("central-controle");
        engr.set_child(Some(&ui_icon("engrenagem", "#8E8F99", 16)));
        engr.set_tooltip_text(Some(tr("Agent settings")));
        engr.set_valign(Align::Center);
        rodape.append(&engr);
        lateral.append(&rodape);

        // ---- the middle
        let raiz = caixa(Orientation::Horizontal, 0, "");
        raiz.set_vexpand(true);
        let meio = caixa(Orientation::Vertical, 0, "conversas-meio");
        meio.set_hexpand(true);
        let mc = caixa(Orientation::Horizontal, 10, "conversas-cab");
        // (a narrow window: the side bar's button and the agent's star before the title, the agent under it)
        let abrir_lista_b = gtk::Button::new();
        abrir_lista_b.add_css_class("central-controle");
        abrir_lista_b.set_child(Some(&ui_icon("painel-esquerda", "#8E8F99", 16)));
        abrir_lista_b.set_tooltip_text(Some(tr("Conversations")));
        abrir_lista_b.set_valign(Align::Center);
        mc.append(&abrir_lista_b);
        let faisca = estrela(13, "#A5A1E2");
        faisca.set_valign(Align::Center);
        mc.append(&faisca);
        let tcol = caixa(Orientation::Vertical, 1, "");
        tcol.set_valign(Align::Center);
        let titulo = rotulo(tr("New conversation"), "conversas-titulo");
        titulo.set_ellipsize(gtk::pango::EllipsizeMode::End);
        titulo.set_xalign(0.0);
        tcol.append(&titulo);
        let subtitulo = rotulo("", "central-agente-sub");
        subtitulo.set_xalign(0.0);
        subtitulo.set_ellipsize(gtk::pango::EllipsizeMode::End);
        tcol.append(&subtitulo);
        mc.append(&tcol);
        let chip_agente_box = caixa(Orientation::Horizontal, 5, "rotinas-chip");
        chip_agente_box.add_css_class("conversas-chip-agente");
        chip_agente_box.set_valign(Align::Center);
        let chip_tile = caixa(Orientation::Horizontal, 0, "");
        chip_agente_box.append(&chip_tile);
        let chip_agente = rotulo(tr("Agent"), "");
        chip_agente.set_ellipsize(gtk::pango::EllipsizeMode::End);
        chip_agente.set_max_width_chars(24);
        chip_agente_box.append(&chip_agente);
        mc.append(&chip_agente_box);
        let esp = caixa(Orientation::Horizontal, 0, "");
        esp.set_hexpand(true);
        mc.append(&esp);
        let fixar = gtk::Button::new();
        fixar.add_css_class("central-controle");
        fixar.set_child(Some(&ui_icon("alfinete", "#8E8F99", 16)));
        fixar.set_tooltip_text(Some(tr("Pin")));
        mc.append(&fixar);
        let apagar = gtk::Button::new();
        apagar.add_css_class("central-controle");
        apagar.set_child(Some(&ui_icon("lixeira", "#8E8F99", 16)));
        apagar.set_tooltip_text(Some(tr("Delete conversation")));
        mc.append(&apagar);
        let painel_b = gtk::ToggleButton::new();
        painel_b.add_css_class("central-controle");
        painel_b.set_child(Some(&ui_icon("painel-direita", "#8E8F99", 16)));
        painel_b.set_tooltip_text(Some(tr("Context")));
        mc.append(&painel_b);
        let controles_meio = controles();
        mc.append(&controles_meio);
        let mch = gtk::WindowHandle::new();
        mch.set_child(Some(&mc));
        meio.append(&mch);
        meio.append(&caixa(Orientation::Horizontal, 0, "central-linha"));
        let vista = gtk::Stack::new();
        vista.set_transition_type(gtk::StackTransitionType::Crossfade);
        vista.set_transition_duration(160);
        vista.set_vexpand(true);
        // a new conversation
        let nv = caixa(Orientation::Vertical, 0, "");
        nv.set_valign(Align::Center);
        // (at most the design's width, narrower when the window is: see limite.rs)
        let saudacao = rotulo("", "conversas-saudacao");
        saudacao.set_wrap(true);
        saudacao.set_justify(gtk::Justification::Center);
        nv.append(&saudacao);
        let pergunta = rotulo(tr("What shall we work on?"), "central-sub");
        pergunta.set_margin_top(6);
        nv.append(&pergunta);
        // the agents: the system's, the ones used last, then all of them (the picker)
        let start_chips = caixa(Orientation::Horizontal, 8, "");
        start_chips.set_halign(Align::Center);
        start_chips.set_margin_top(20);
        // (full names: the chips that don't fit hide, "Todos os agentes" still reaches them)
        crate::fit_row::FitRow::apply(&start_chips, 8);
        nv.append(&start_chips);
        let comp = caixa(Orientation::Vertical, 10, "conversas-compositor");
        comp.set_margin_top(18);
        let nova_form = agent_picker::InputForm::new();
        nova_form.root.set_visible(false);
        comp.append(&nova_form.root);
        let nova_erro = crate::fluxos_ui::error_label();
        comp.append(&nova_erro);
        let nova_texto = gtk::TextView::new();
        nova_texto.add_css_class("conversas-texto");
        nova_texto.set_wrap_mode(gtk::WrapMode::WordChar);
        nova_texto.set_accepts_tab(false);
        let sobre = gtk::Overlay::new();
        sobre.set_child(Some(&nova_texto));
        let ph = rotulo(tr("Ask anything · / commands · @ calls an agent"), "conversas-placeholder");
        ph.set_halign(Align::Start);
        ph.set_valign(Align::Start);
        ph.set_can_target(false);
        sobre.add_overlay(&ph);
        comp.append(&sobre);
        let cl = caixa(Orientation::Horizontal, 6, "");
        let anexar = gtk::Button::new();
        anexar.add_css_class("conversas-ferramenta");
        anexar.set_child(Some(&ui_icon("mais", "#C9CAD1", 14)));
        anexar.set_tooltip_text(Some(tr("Add a file")));
        cl.append(&anexar);
        let mut nova_grupos = Vec::new();
        let mut nomes_chips = Vec::new();
        for (k, nome, glifo, _) in GRUPOS {
            let b = gtk::ToggleButton::new();
            b.add_css_class("conversas-ferramenta");
            b.set_tooltip_text(Some(tr(nome)));
            let bc = caixa(Orientation::Horizontal, 6, "");
            bc.append(&ui_icon(glifo, "#C9CAD1", 13));
            let n = rotulo(tr(nome), "");
            n.set_ellipsize(gtk::pango::EllipsizeMode::End);
            bc.append(&n);
            nomes_chips.push(n);
            b.set_child(Some(&bc));
            b.set_active(true);
            cl.append(&b);
            nova_grupos.push((*k, b));
        }
        let ce = caixa(Orientation::Horizontal, 0, "");
        ce.set_hexpand(true);
        cl.append(&ce);
        let modelo_chip = rotulo(&format!("● {} · {}", conversa::guess_model(), tr("local")), "conversas-modelo");
        modelo_chip.set_valign(Align::Center);
        modelo_chip.set_ellipsize(gtk::pango::EllipsizeMode::End);
        cl.append(&modelo_chip);
        let enviar = gtk::Button::new();
        enviar.add_css_class("conversas-enviar");
        enviar.set_child(Some(&ui_icon("enviar", "#1C1D24", 16)));
        cl.append(&enviar);
        comp.append(&cl);
        nv.append(&comp);
        let dicas = caixa(Orientation::Horizontal, 8, "");
        dicas.set_margin_top(8);
        let d1 = rotulo(tr("Enter sends · Shift+Enter breaks the line"), "central-agente-sub");
        d1.set_hexpand(true);
        d1.set_xalign(0.0);
        d1.set_ellipsize(gtk::pango::EllipsizeMode::End);
        dicas.append(&d1);
        dicas.append(&ui_icon("escudo", "#8ED2C3", 13));
        let d2 = rotulo(tr("local: nothing leaves the computer"), "central-agente-sub");
        d2.set_ellipsize(gtk::pango::EllipsizeMode::End);
        dicas.append(&d2);
        nv.append(&dicas);
        // (as many cards per row as fit, each at least 220 px: one column when narrow, more on a wide window)
        let sugestoes = gtk::FlowBox::new();
        sugestoes.set_selection_mode(gtk::SelectionMode::None);
        sugestoes.set_column_spacing(12);
        sugestoes.set_row_spacing(12);
        sugestoes.set_homogeneous(true);
        sugestoes.set_min_children_per_line(1);
        sugestoes.set_max_children_per_line(6);
        sugestoes.set_hexpand(true);
        sugestoes.set_margin_top(28);
        nv.append(&sugestoes);
        let rodape_d = caixa(Orientation::Horizontal, 8, "");
        rodape_d.set_halign(Align::Center);
        rodape_d.set_margin_top(28);
        for (tecla, t) in [("/", tr("commands")), ("@", tr("mention")), ("Super+Espaço", tr("ask from anywhere"))] {
            rodape_d.append(&rotulo(tecla, "visao-atalho"));
            let r = rotulo(t, "central-agente-sub");
            r.set_ellipsize(gtk::pango::EllipsizeMode::End);
            rodape_d.append(&r);
        }
        nv.append(&rodape_d);
        let nvr = gtk::ScrolledWindow::new();
        nvr.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        nvr.set_child(Some(&crate::limite::Limite::new(&nv, if compacta { 620 } else { LEITURA })));
        vista.add_named(&nvr, Some("nova"));
        // a kept conversation
        let conv = Conversa::new();
        conv.modo_app();
        shell.ligar_pedidos(&conv);
        // on screen: the agents center's window in front, on this page, with a conversation
        let (cr, vw) = (conv.raiz.downgrade(), vista.downgrade());
        conv.set_visivel(move || {
            cr.upgrade().is_some_and(|r| r.is_mapped() && r.root().and_downcast::<gtk::Window>().is_some_and(|w| w.is_active()))
                && vw.upgrade().is_some_and(|v| v.visible_child_name().as_deref() == Some("conversa"))
        });
        let cw = caixa(Orientation::Vertical, 0, "");
        cw.append(&conv.raiz);
        // (the column grows with the room the side panels leave — design "Central · conversa responsiva")
        vista.add_named(&crate::limite::Limite::with_rule(&cw, widths::reading_column), Some("conversa"));
        meio.append(&vista);
        raiz.append(&meio);

        // ---- the context, on the right
        let painel = caixa(Orientation::Vertical, 0, "conversas-painel");
        let painel_base = if compacta { 300 } else { 340 };
        painel.set_size_request(painel_base, -1);
        // (its own width, not a share of the free room: a child that expands must not widen it)
        painel.set_hexpand(false);
        let pc = caixa(Orientation::Horizontal, 14, "conversas-cab");
        let mut abas: Vec<gtk::ToggleButton> = Vec::new();
        for t in [tr("Context"), tr("Files"), tr("Canvas"), tr("Changes"), tr("Terminal"), tr("Browser")] {
            let b = gtk::ToggleButton::with_label(t);
            b.add_css_class("ajustes-aba");
            if let Some(f) = abas.first() {
                b.set_group(Some(f));
            }
            pc.append(&b);
            abas.push(b);
        }
        // (code mode — the agent changed files in a git repository — shows Changes first, Files and Terminal, as the
        // design "Chat · código"; otherwise Context, Files and Canvas; Terminal once the agent ran commands)
        abas[3].set_visible(false);
        pc.reorder_child_after(&abas[3], None::<&gtk::Widget>);
        abas[4].set_visible(false);
        // (web mode — the agent read pages — shows the Browser first, then Canvas and Context, as the design
        // "Chat · navegador do agente")
        abas[5].set_visible(false);
        pc.reorder_child_after(&abas[5], Some(&abas[3]));
        // (Canvas: the latest diagram of the conversation, once there is one)
        abas[2].set_sensitive(false);
        abas[2].set_tooltip_text(Some(tr("A diagram in the conversation opens here")));
        let pe = caixa(Orientation::Horizontal, 0, "");
        pe.set_hexpand(true);
        pc.append(&pe);
        let dobrar = gtk::Button::new();
        dobrar.add_css_class("central-controle");
        dobrar.set_child(Some(&ui_icon("painel-direita", "#8E8F99", 16)));
        dobrar.set_tooltip_text(Some(tr("Fold the context")));
        dobrar.set_valign(Align::Center);
        // (Canvas and Browser: the canvas may take most of the window — design "Central · canvas expandido")
        let expandir = gtk::Button::new();
        expandir.add_css_class("central-controle");
        expandir.set_child(Some(&ui_icon("expandir", "#8E8F99", 16)));
        expandir.set_tooltip_text(Some(tr("Expand the canvas")));
        expandir.set_valign(Align::Center);
        expandir.set_visible(false);
        pc.append(&expandir);
        pc.append(&dobrar);
        let controles_painel = controles();
        pc.append(&controles_painel);
        let pch = gtk::WindowHandle::new();
        pch.set_child(Some(&pc));
        painel.append(&pch);
        let painel_stack = gtk::Stack::new();
        painel_stack.set_vexpand(true);
        // (only the tab on screen sets the context's width: the Browser's wide rows would hold a half-screen window
        // wider than its column — suite run 53, §80)
        painel_stack.set_hhomogeneous(false);
        let ctx = caixa(Orientation::Vertical, 0, "");
        let secao = |t: &str, glifo: &str| {
            let l = caixa(Orientation::Horizontal, 8, "");
            l.set_margin_top(16);
            l.set_margin_bottom(10);
            l.append(&ui_icon(glifo, "#8E8F99", 13));
            l.append(&rotulo(t, "central-painel-secao"));
            l
        };
        ctx.append(&secao(tr("IN THIS CONVERSATION"), "cabo"));
        let contexto = caixa(Orientation::Vertical, 0, "central-cartao");
        ctx.append(&contexto);
        let add_ctx = gtk::Button::new();
        add_ctx.add_css_class("central-adicionar");
        let ac = caixa(Orientation::Horizontal, 8, "");
        ac.append(&ui_icon("mais", "#C9CAD1", 13));
        ac.append(&rotulo(tr("Add context"), ""));
        add_ctx.set_child(Some(&ac));
        add_ctx.set_halign(Align::Start);
        add_ctx.set_margin_top(10);
        ctx.append(&add_ctx);
        ctx.append(&secao(tr("TOOLS"), "ajustes"));
        let ferramentas = caixa(Orientation::Vertical, 0, "central-cartao");
        ctx.append(&ferramentas);
        ctx.append(&secao(tr("MEMORY USED"), "cerebro"));
        let memoria = caixa(Orientation::Vertical, 6, "central-cartao");
        memoria.add_css_class("memoria-info");
        ctx.append(&memoria);
        let crol = gtk::ScrolledWindow::new();
        crol.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        crol.set_child(Some(&ctx));
        painel_stack.add_named(&crol, Some("contexto"));
        let arquivos = caixa(Orientation::Vertical, 0, "central-cartao");
        arquivos.set_margin_top(16);
        let arol = gtk::ScrolledWindow::new();
        arol.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        arol.set_child(Some(&arquivos));
        painel_stack.add_named(&arol, Some("arquivos"));
        let canvas_box = caixa(Orientation::Vertical, 0, "");
        canvas_box.set_margin_top(16);
        // (room for the panel's scrollbar at the right)
        canvas_box.set_margin_end(8);
        let crol = gtk::ScrolledWindow::new();
        crol.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        crol.set_child(Some(&canvas_box));
        painel_stack.add_named(&crol, Some("canvas"));
        let alteracoes_box = caixa(Orientation::Vertical, 0, "");
        alteracoes_box.set_margin_top(16);
        alteracoes_box.set_margin_end(8);
        let alrol = gtk::ScrolledWindow::new();
        alrol.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        alrol.set_child(Some(&alteracoes_box));
        painel_stack.add_named(&alrol, Some("alteracoes"));
        let terminal_box = caixa(Orientation::Vertical, 10, "");
        terminal_box.set_margin_top(16);
        terminal_box.set_margin_end(8);
        let trol = gtk::ScrolledWindow::new();
        trol.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        trol.set_child(Some(&terminal_box));
        painel_stack.add_named(&trol, Some("terminal"));
        let navegador = crate::agent_browser::AgentBrowser::new();
        painel_stack.add_named(&navegador.raiz, Some("navegador"));
        painel.append(&painel_stack);
        let uso = caixa(Orientation::Vertical, 8, "central-cartao");
        uso.add_css_class("memoria-info");
        uso.set_margin_top(12);
        let linha_uso = |n: &str| {
            let l = caixa(Orientation::Horizontal, 8, "");
            let a = rotulo(n, "central-agente-sub");
            a.set_hexpand(true);
            a.set_xalign(0.0);
            l.append(&a);
            let v = rotulo("", "memoria-info-valor");
            l.append(&v);
            uso.append(&l);
            v
        };
        let tokens_v = linha_uso(tr("This conversation"));
        let tokens_barra = gtk::LevelBar::for_interval(0.0, 1.0);
        tokens_barra.add_css_class("conversas-barra");
        uso.append(&tokens_barra);
        let modelo_v = linha_uso(tr("Model"));
        modelo_v.add_css_class("teal");
        let custo = linha_uso(tr("Cost"));
        custo.set_text("R$ 0,00");
        painel.append(&uso);
        raiz.append(&painel);
        // the context folded into its tabs (a medium window): each opens it on its tab
        let faixa_abas = caixa(Orientation::Vertical, 6, "conversas-faixa");
        faixa_abas.set_size_request(52, -1);
        faixa_abas.set_hexpand(false);
        let mut botoes_faixa = Vec::new();
        // (one per tab of the context — enabled as its tab is — and the agent's browser, still to come)
        for (glifo, nome, pronta) in [("clipe", tr("Context"), true), ("pasta", tr("Files"), true), ("camadas", tr("Canvas"), true), ("codigo", tr("Changes"), true), ("terminal", tr("Terminal"), true), ("globo", tr("The agent's browser"), true)] {
            let b = gtk::Button::new();
            b.add_css_class("conversas-faixa-aba");
            b.set_child(Some(&ui_icon(glifo, "#8E8F99", 16)));
            b.set_tooltip_text(Some(&if pronta { nome.to_string() } else { trf("{page} (not in this version yet)", &[("page", nome.to_string())]) }));
            b.set_sensitive(pronta);
            b.set_halign(Align::Center);
            faixa_abas.append(&b);
            botoes_faixa.push((glifo, b));
        }
        // (Changes first, as in the header)
        faixa_abas.reorder_child_after(&botoes_faixa[3].1, None::<&gtk::Widget>);
        let fe = caixa(Orientation::Vertical, 0, "");
        fe.set_vexpand(true);
        faixa_abas.append(&fe);
        let desdobrar = gtk::Button::new();
        desdobrar.add_css_class("conversas-faixa-aba");
        desdobrar.set_child(Some(&ui_icon("painel-direita", "#8E8F99", 16)));
        desdobrar.set_tooltip_text(Some(tr("Open the context")));
        desdobrar.set_halign(Align::Center);
        faixa_abas.append(&desdobrar);
        raiz.append(&faixa_abas);
        // (dims the conversation under a sheet; a click on it closes the sheet)
        let veu = caixa(Orientation::Vertical, 0, "conversas-veu");
        veu.set_visible(false);

        let c = Rc::new(Conversas {
            nova_b: nova.clone(), raiz, lateral, shell: Rc::downgrade(shell), ir: RefCell::default(), recolher: RefCell::default(), recolhida: Cell::new(false),
            faixa: Cell::new(Faixa::de(largura_janela)), largura: Cell::new(largura_janela), na_pagina: Cell::new(false), painel_aberto: Cell::new(Faixa::de(largura_janela) == Faixa::Larga), arrumando: Cell::new(false),
            folhas: RefCell::default(), veu, casa_lateral: RefCell::default(), casa_painel: RefCell::default(), faixa_abas, faixa_botoes: botoes_faixa.clone(), controles_painel, abrir_lista_b, faisca, subtitulo, nomes_chips, ja_mostrou: Cell::new(false), busca, lista, rodape_tokens,
            titulo, chip_agente, fixar, apagar: apagar.clone(), painel_b: painel_b.clone(), vista, conv, controles_meio, saudacao, nova_texto, nova_grupos, sugestoes, painel, abas, painel_stack, contexto, ferramentas,
            memoria, arquivos, tokens_v, tokens_barra, modelo_v, marca: Cell::new((0, 0)), canvas_box, html: crate::html_canvas::HtmlCanvas::new(), alteracoes_box, terminal_box, navegador, painel_base, largura_contexto: Cell::new(guardada("contexto")), largura_lista: Cell::new(guardada("lista")), canvas_width: Cell::new(guardada("canvas")), browser_width: Cell::new(guardada("navegador")), expanded: Cell::new(false), list_closed_before: Cell::new(false), expand_button: expandir.clone(), alca_contexto: RefCell::default(), alca_lista: RefCell::default(), uso: uso.clone(), repo: RefCell::default(), commit_feito: RefCell::default(), previa: RefCell::default(), preenchendo: Cell::new(false),
            start_chips, start_agent: RefCell::default(), start_picker: RefCell::default(), nova_form, nova_ph: ph.clone(), chip_agente_box, chip_tile, ao_executar: RefCell::default(), nova_erro,
        });
        // the edges of the context and the side bar (the agents center puts the side bar's beside it)
        let (ac, al) = c.alcas();
        c.raiz.insert_child_after(&ac, c.painel.prev_sibling().as_ref());
        *c.alca_contexto.borrow_mut() = Some(ac);
        *c.alca_lista.borrow_mut() = Some(al);
        // the conversation's agent: the header's chip follows it; the picker's other choices are the page's
        let w = Rc::downgrade(&c);
        c.conv.on_agent(move |a| {
            if let Some(c) = w.upgrade() {
                c.show_header_agent(a);
                c.lista_render();
            }
        });
        c.show_header_agent(None);
        let w = Rc::downgrade(&c);
        *c.conv.on_picked_other.borrow_mut() = Some(Box::new(move |x| {
            if let Some(c) = w.upgrade() {
                c.picked(x);
            }
        }));
        c.abas[0].set_active(true);
        // a request about an element of the canvas's page goes to this conversation's agent
        let w = Rc::downgrade(&c.conv);
        *c.html.on_ask.borrow_mut() = Some(Box::new(move |msg| {
            if let Some(cv) = w.upgrade() {
                cv.ask(&msg);
            }
        }));
        // a page from the conversation (a source, a login): the Browser tab, open, with that page
        let w = Rc::downgrade(&c);
        *c.conv.on_page.borrow_mut() = Some(Box::new(move |url, control| {
            if let Some(c) = w.upgrade() {
                c.abas[5].set_visible(true);
                c.abas[5].set_active(true);
                c.abrir_painel(true);
                c.navegador.show(url, control);
            }
        }));
        // the browser's "Pausar" stops the agent of this conversation
        let w = Rc::downgrade(&c.conv);
        *c.navegador.on_pause.borrow_mut() = Some(Box::new(move || {
            if let Some(cv) = w.upgrade() {
                cv.stop(true);
            }
        }));
        for (i, b) in c.abas.iter().enumerate() {
            let w = Rc::downgrade(&c);
            b.connect_toggled(move |b| {
                if let Some(c) = w.upgrade().filter(|_| b.is_active()) {
                    // (another tab puts an expanded canvas back first)
                    c.expand(false);
                    c.painel_stack.set_visible_child_name(["contexto", "arquivos", "canvas", "alteracoes", "terminal", "navegador"][i]);
                    c.ajustar_painel();
                    c.mark_expand_button();
                    // (the canvas with a page needs the height: no usage under it)
                    c.uso.set_visible(i < 2 || (i == 2 && c.html.file().is_none()));
                    if i == 2 {
                        c.canvas_render();
                    }
                    if i == 3 {
                        c.alteracoes_render();
                    }
                    if i == 4 {
                        c.terminal_render();
                    }
                    c.marcar_faixa();
                }
            });
        }
        for (id, b) in nav_botoes {
            let w = Rc::downgrade(&c);
            b.connect_clicked(move |_| {
                if let Some(c) = w.upgrade() {
                    if id == "conversas" {
                        c.nova();
                    } else if let Some(f) = c.ir.borrow().as_ref() {
                        f(id);
                    }
                }
            });
        }
        let w = Rc::downgrade(&c);
        recolher_b.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                c.abrir_lista(false);
            }
        });
        let w = Rc::downgrade(&c);
        engr.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                if let Some(f) = c.ir.borrow().as_ref() {
                    f("ajustes");
                }
            }
        });
        let w = Rc::downgrade(&c);
        nova.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                c.nova();
            }
        });
        let w = Rc::downgrade(&c);
        c.busca.connect_changed(move |_| {
            if let Some(c) = w.upgrade() {
                c.lista_render();
            }
        });
        let w = Rc::downgrade(&c);
        c.fixar.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                let id = c.conv.id();
                let mut m = conversa::meta(&id);
                if m["titulo"].is_string() {
                    m["fixada"] = json!(!m["fixada"].as_bool().unwrap_or(false));
                    conversa::salvar_meta(&id, &m);
                    c.lista_render();
                    c.cabecalho();
                }
            }
        });
        let w = Rc::downgrade(&c);
        apagar.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                let id = c.conv.id();
                c.conv.nova();
                let _ = std::fs::remove_dir_all(conversa::conversas_dir().join(&id));
                c.nova();
            }
        });
        let w = Rc::downgrade(&c);
        painel_b.connect_toggled(move |b| {
            if let Some(c) = w.upgrade().filter(|c| !c.arrumando.get()) {
                c.abrir_painel(b.is_active());
            }
        });
        let w = Rc::downgrade(&c);
        expandir.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                c.expand(!c.expanded.get());
            }
        });
        let w = Rc::downgrade(&c);
        dobrar.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                c.abrir_painel(false);
            }
        });
        let w = Rc::downgrade(&c);
        desdobrar.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                c.abrir_painel(true);
            }
        });
        for (i, (_, b)) in botoes_faixa.iter().enumerate() {
            if let Some(a) = c.abas.get(i) {
                a.bind_property("sensitive", b, "sensitive").sync_create().build();
                a.bind_property("visible", b, "visible").sync_create().build();
            }
            let w = Rc::downgrade(&c);
            b.connect_clicked(move |_| {
                if let Some(c) = w.upgrade() {
                    if let Some(a) = c.abas.get(i) {
                        a.set_active(true);
                    }
                    c.abrir_painel(true);
                }
            });
            crate::central::posicao_debug_sempre(&format!("conversas faixa {i}"), b.upcast_ref());
        }
        crate::central::posicao_debug_sempre("conversas faixa abrir", desdobrar.upcast_ref());
        crate::central::posicao_debug_sempre("conversas dobrar contexto", dobrar.upcast_ref());
        crate::central::posicao_debug_sempre("conversas abrir lista", c.abrir_lista_b.upcast_ref());
        crate::central::posicao_debug_sempre("conversas contexto", painel_b.upcast_ref());
        let w = Rc::downgrade(&c);
        c.abrir_lista_b.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                let aberta = !c.recolhida.get();
                c.abrir_lista(!aberta);
            }
        });
        let fechar = gtk::GestureClick::new();
        let w = Rc::downgrade(&c);
        fechar.connect_released(move |_, _, _, _| {
            if let Some(c) = w.upgrade() {
                c.fechar_folhas();
            }
        });
        c.veu.add_controller(fechar);
        let w = Rc::downgrade(&c);
        crate::diagram_block::on_new(move || {
            if let Some(c) = w.upgrade() {
                c.abas[2].set_sensitive(true);
                c.abas[2].set_visible(true);
                if c.abas[2].is_active() {
                    c.canvas_render();
                }
            }
        });
        // (measures the middle: the context and then the side bar give way when it gets narrow)
        let medidor = gtk::DrawingArea::new();
        medidor.set_can_target(false);
        medidor.set_hexpand(true);
        c.meio_medidor(&medidor);
        let w = Rc::downgrade(&c);
        medidor.connect_resize(move |_, largura, _| {
            if let Some(c) = w.upgrade() {
                glib::idle_add_local_once(move || c.caber(largura));
            }
        });
        // the context shows with a conversation (a new one has nothing to show yet), open in a wide window
        let w = Rc::downgrade(&c);
        c.vista.connect_visible_child_name_notify(move |v| {
            if let Some(c) = w.upgrade() {
                let em_conversa = v.visible_child_name().as_deref() == Some("conversa");
                c.painel_aberto.set(em_conversa && c.faixa.get() == Faixa::Larga);
                c.arrumar();
            }
        });
        c.painel.set_visible(false);
        // the new conversation's composer: Enter sends, Shift+Enter breaks the line
        let k = gtk::EventControllerKey::new();
        let w = Rc::downgrade(&c);
        k.connect_key_pressed(move |_, tecla, _, mods| {
            if matches!(tecla, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter) && !mods.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
                if let Some(c) = w.upgrade() {
                    c.enviar_nova();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        c.nova_texto.add_controller(k);
        let phw = ph.downgrade();
        c.nova_texto.buffer().connect_changed(move |b| {
            if let Some(p) = phw.upgrade() {
                p.set_visible(b.char_count() == 0);
            }
        });
        let w = Rc::downgrade(&c);
        enviar.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                c.enviar_nova();
            }
        });
        for (_, b) in &c.nova_grupos {
            let w = Rc::downgrade(&c);
            b.connect_toggled(move |_| {
                if let Some(c) = w.upgrade().filter(|c| !c.preenchendo.get()) {
                    let d = c.desligadas_da_nova();
                    c.conv.set_desligadas(d);
                }
            });
        }
        let w = Rc::downgrade(&c);
        anexar.connect_clicked(move |_| {
            let Some(c) = w.upgrade() else { return };
            let w = Rc::downgrade(&c);
            crate::arquivos::seletor::abrir_arquivo(tr("Agents"), tr("Add a file to the message"), Some(tr("Add")), Vec::new(), None, move |p| {
                if let Some(c) = w.upgrade() {
                    c.nova_texto.buffer().insert_at_cursor(&format!("{} ", p.display()));
                }
            });
        });
        let w = Rc::downgrade(&c);
        add_ctx.connect_clicked(move |_| {
            let Some(c) = w.upgrade() else { return };
            let w = Rc::downgrade(&c);
            crate::arquivos::seletor::abrir_arquivo(tr("Agents"), tr("Add context"), Some(tr("Add")), Vec::new(), None, move |p| {
                if let Some(c) = w.upgrade() {
                    c.conv.inserir(&format!("{} ", p.display()));
                }
            });
        });
        c
    }

    /// A file for the next message.
    pub fn anexar(&self, p: std::path::PathBuf) {
        self.conv.add_attachment(p);
    }

    pub fn ao_ir(&self, f: impl Fn(&str) + 'static) {
        *self.ir.borrow_mut() = Some(Box::new(f));
    }

    fn meio_medidor(&self, m: &gtk::DrawingArea) {
        if let Some(meio) = self.vista.parent().and_downcast::<gtk::Box>() {
            meio.prepend(m);
        }
    }

    /// The middle is `largura` px wide: the composer's chips keep only their icons below 640 px (their names in the
    /// tooltips). The side bars follow the window's band (`largura_janela`).
    fn caber(&self, largura: i32) {
        for n in &self.nomes_chips {
            n.set_visible(largura >= 640);
        }
    }

    /// The window is `largura` px wide: a new band puts the side bars where it wants them (the person's own choices
    /// hold until the band changes).
    pub fn largura_janela(&self, largura: i32) {
        let antes = self.largura.replace(largura);
        // (for the session test: the narrow header's buttons move with the window's width)
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() && antes != largura && self.faixa.get() == Faixa::Estreita {
            crate::central::posicao_debug("conversas contexto", self.painel_b.upcast_ref());
            crate::central::posicao_debug("conversas abrir lista", self.abrir_lista_b.upcast_ref());
        }
        let f = Faixa::de(largura);
        if f == self.faixa.get() {
            if antes != largura {
                self.ajustar_lista();
                self.ajustar_painel();
            }
            return;
        }
        self.faixa.set(f);
        self.expanded.set(false);
        self.recolhida.set(f != Faixa::Larga);
        let em_conversa = self.vista.visible_child_name().as_deref() == Some("conversa");
        self.painel_aberto.set(em_conversa && f == Faixa::Larga);
        self.arrumar();
    }

    /// The agents center's overlay: the side bar and the context become sheets over the page in a narrow window.
    pub fn set_folhas(self: &Rc<Self>, sobre: &gtk::Overlay) {
        *self.casa_lateral.borrow_mut() = self.lateral.parent().and_downcast::<gtk::Box>().map(|p| (p, self.lateral.prev_sibling()));
        *self.casa_painel.borrow_mut() = self.painel.parent().and_downcast::<gtk::Box>().map(|p| (p, self.painel.prev_sibling()));
        self.veu.set_hexpand(true);
        self.veu.set_vexpand(true);
        sobre.add_overlay(&self.veu);
        *self.folhas.borrow_mut() = Some(sobre.clone());
        // Esc closes a sheet
        let k = gtk::EventControllerKey::new();
        k.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        k.connect_key_pressed(move |_, tecla, _, _| {
            match w.upgrade() {
                Some(c) if tecla == gtk::gdk::Key::Escape && c.veu.is_visible() => {
                    c.fechar_folhas();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        sobre.add_controller(k);
        self.arrumar();
    }

    /// The page shows (`true`) or another page of the center does.
    pub fn na_pagina(&self, sim: bool, de_novo: bool) {
        self.na_pagina.set(sim);
        if sim {
            // (coming in: the side bar as the band wants it; the rail's own button again opens it)
            self.recolhida.set(if de_novo { false } else { self.faixa.get() != Faixa::Larga });
        }
        self.arrumar();
    }

    /// Opens or closes the side bar (in a narrow window, as a sheet).
    pub fn abrir_lista(&self, abrir: bool) {
        self.recolhida.set(!abrir);
        if abrir && self.faixa.get() == Faixa::Estreita {
            self.painel_aberto.set(false);
        }
        self.arrumar();
    }

    fn abrir_painel(&self, abrir: bool) {
        self.painel_aberto.set(abrir);
        if abrir && self.faixa.get() == Faixa::Estreita {
            self.recolhida.set(true);
        }
        self.arrumar();
    }

    fn fechar_folhas(&self) {
        if self.faixa.get() == Faixa::Estreita {
            self.recolhida.set(true);
            self.painel_aberto.set(false);
            self.arrumar();
        }
    }

    /// A side bar in its place in the page, or a sheet over it.
    fn por_em_folha(&self, w: &gtk::Box, folha: bool, lado: Align, casa: &RefCell<Option<(gtk::Box, Option<gtk::Widget>)>>) {
        let Some(sobre) = self.folhas.borrow().clone() else { return };
        let em_folha = w.parent().is_some_and(|p| p == *sobre.upcast_ref::<gtk::Widget>());
        if folha && !em_folha {
            if let Some(p) = w.parent().and_downcast::<gtk::Box>() {
                p.remove(w);
            }
            w.set_halign(lado);
            w.add_css_class("conversas-folha");
            sobre.add_overlay(w);
        } else if !folha && em_folha {
            sobre.remove_overlay(w);
            w.set_halign(Align::Fill);
            w.remove_css_class("conversas-folha");
            if let Some((p, antes)) = casa.borrow().as_ref() {
                p.insert_child_after(w, antes.as_ref());
            }
        }
    }

    /// The Canvas tab (design "Chat · diagramas e fórmulas"): the latest diagram's source, edited here and drawn again
    /// in the conversation; a change asked of the agent; its direction and corners; exported or put in the open document.
    fn canvas_render(self: &Rc<Self>) {
        while let Some(x) = self.canvas_box.first_child() {
            self.canvas_box.remove(&x);
        }
        // a page the agent wrote: the live HTML canvas
        if self.html.file().is_some() {
            if let Some(p) = self.html.raiz.parent().and_downcast::<gtk::Box>() {
                p.remove(&self.html.raiz);
            }
            self.canvas_box.append(&self.html.raiz);
            return;
        }
        let Some(d) = crate::diagram_block::current() else {
            let v = rotulo(tr("No diagram in this conversation yet."), "central-agente-sub");
            v.set_xalign(0.0);
            self.canvas_box.append(&v);
            return;
        };
        let secao = |t: &str, glifo: &str| {
            let l = caixa(Orientation::Horizontal, 8, "");
            l.set_margin_top(16);
            l.set_margin_bottom(10);
            if glifo == "spark" { l.append(&estrela(12, "#A5A1E2")) } else { l.append(&ui_icon(glifo, "#8E8F99", 13)) }
            l.append(&rotulo(t, "central-painel-secao"));
            l
        };
        // the source, editable
        let card = caixa(Orientation::Vertical, 0, "md-bloco");
        let cab = caixa(Orientation::Horizontal, 8, "md-bloco-cab");
        cab.append(&ui_icon("codigo", "#8E8F99", 16));
        cab.append(&rotulo("Mermaid", "md-bloco-titulo"));
        let det = rotulo(&format!("{}.mmd", tr("diagram")), "md-bloco-detalhe");
        det.set_hexpand(true);
        det.set_xalign(0.0);
        cab.append(&det);
        let copiar = gtk::Button::new();
        copiar.add_css_class("md-bloco-botao");
        copiar.set_child(Some(&rotulo(tr("Copy"), "")));
        copiar.set_valign(Align::Center);
        cab.append(&copiar);
        card.append(&cab);
        card.append(&caixa(Orientation::Horizontal, 0, "md-bloco-div"));
        let tv = gtk::TextView::new();
        tv.add_css_class("canvas-fonte");
        tv.set_monospace(true);
        tv.set_wrap_mode(gtk::WrapMode::None);
        tv.set_left_margin(12);
        tv.set_right_margin(12);
        tv.set_top_margin(10);
        tv.set_bottom_margin(10);
        tv.buffer().set_text(&d.src.borrow());
        let tr_ = gtk::ScrolledWindow::new();
        tr_.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        tr_.set_propagate_natural_height(true);
        tr_.set_child(Some(&tv));
        tr_.add_css_class("md-bloco-rolagem");
        card.append(&tr_);
        self.canvas_box.append(&card);
        let b = tv.buffer();
        copiar.connect_clicked(move |x| x.clipboard().set_text(&b.text(&b.start_iter(), &b.end_iter(), false)));
        // (edited: drawn again a moment after the typing stops; a source that does not read keeps the last drawing)
        let espera: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        let (dw, cw) = (Rc::downgrade(&d), card.downgrade());
        tv.buffer().connect_changed(move |b| {
            if let Some(t) = espera.borrow_mut().take() {
                t.remove();
            }
            let (dw, cw, b, e2) = (dw.clone(), cw.clone(), b.clone(), espera.clone());
            *espera.borrow_mut() = Some(glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
                e2.borrow_mut().take();
                let (Some(d), Some(c)) = (dw.upgrade(), cw.upgrade()) else { return };
                let ok = d.set_source(&b.text(&b.start_iter(), &b.end_iter(), false));
                if ok { c.remove_css_class("erro") } else { c.add_css_class("erro") }
            }));
        });
        // a change asked of the agent
        self.canvas_box.append(&secao(tr("ASK FOR A CHANGE IN THE DIAGRAM"), "spark"));
        let pedido = gtk::Entry::new();
        pedido.add_css_class("central-busca-campo");
        pedido.add_css_class("canvas-pedido");
        pedido.set_placeholder_text(Some(tr("add a review step after Execute")));
        let w = Rc::downgrade(self);
        let dw = Rc::downgrade(&d);
        pedido.connect_activate(move |e| {
            let (Some(c), Some(d)) = (w.upgrade(), dw.upgrade()) else { return };
            let t = e.text().trim().to_string();
            if t.is_empty() {
                return;
            }
            e.set_text("");
            let src = d.src.borrow().clone();
            c.conv.ask(&format!("{}\n```mermaid\n{src}\n```", trf("Change this diagram: {change}. Answer with the whole new diagram in mermaid.", &[("change", t)])));
        });
        crate::central::posicao_debug("canvas pedido", pedido.upcast_ref());
        self.canvas_box.append(&pedido);
        // how it looks
        self.canvas_box.append(&secao(tr("APPEARANCE"), "paleta"));
        let ap = caixa(Orientation::Vertical, 2, "central-cartao");
        let linha = |nome: &str, ctl: &gtk::Widget| {
            let r = caixa(Orientation::Horizontal, 8, "conversas-ctx");
            let n = rotulo(nome, "central-limite-nome");
            n.set_hexpand(true);
            n.set_xalign(0.0);
            r.append(&n);
            ctl.set_valign(Align::Center);
            r.append(ctl);
            r
        };
        let dir = caixa(Orientation::Horizontal, 2, "tabela-segmentado");
        let lr = gtk::ToggleButton::new();
        lr.set_child(Some(&ui_icon("seta-direita", "#C9CAD1", 14)));
        let td = gtk::ToggleButton::new();
        td.set_child(Some(&ui_icon("seta-baixo", "#C9CAD1", 14)));
        for x in [&lr, &td] {
            x.add_css_class("tabela-aba");
            dir.append(x);
        }
        td.set_group(Some(&lr));
        if d.horizontal() { lr.set_active(true) } else { td.set_active(true) }
        let (dw, tvw) = (Rc::downgrade(&d), tv.downgrade());
        lr.connect_toggled(move |x| {
            let (Some(d), Some(tv)) = (dw.upgrade(), tvw.upgrade()) else { return };
            d.set_horizontal(x.is_active());
            tv.buffer().set_text(&d.src.borrow());
        });
        crate::central::posicao_debug("canvas direcao vertical", td.upcast_ref());
        ap.append(&linha(tr("Direction"), dir.upcast_ref()));
        let cores = gtk::Switch::new();
        cores.add_css_class("cfg-switch");
        cores.set_active(!d.mono.get());
        let dw = Rc::downgrade(&d);
        cores.connect_state_set(move |_, on| {
            if let Some(d) = dw.upgrade() {
                d.set_mono(!on);
            }
            glib::Propagation::Proceed
        });
        ap.append(&linha(tr("Theme colours"), cores.upcast_ref()));
        let cantos = caixa(Orientation::Horizontal, 2, "tabela-segmentado");
        let retos = gtk::ToggleButton::with_label(tr("Square"));
        let sutis = gtk::ToggleButton::with_label(tr("Soft"));
        for x in [&retos, &sutis] {
            x.add_css_class("tabela-aba");
            cantos.append(x);
        }
        sutis.set_group(Some(&retos));
        if d.square.get() { retos.set_active(true) } else { sutis.set_active(true) }
        let dw = Rc::downgrade(&d);
        retos.connect_toggled(move |x| {
            if let Some(d) = dw.upgrade() {
                d.set_square(x.is_active());
            }
        });
        ap.append(&linha(tr("Corners"), cantos.upcast_ref()));
        self.canvas_box.append(&ap);
        // export
        self.canvas_box.append(&secao(tr("EXPORT"), "download"));
        let ex = caixa(Orientation::Horizontal, 8, "");
        ex.set_homogeneous(true);
        for (fmt, rot) in [("svg", "SVG"), ("png", "PNG"), ("pdf", "PDF")] {
            let b = gtk::Button::new();
            b.add_css_class("midia-acao");
            let c = caixa(Orientation::Horizontal, 6, "");
            c.set_halign(Align::Center);
            c.append(&ui_icon("download", "#C9CAD1", 14));
            c.append(&rotulo(rot, ""));
            b.set_child(Some(&c));
            let dw = Rc::downgrade(&d);
            b.connect_clicked(move |b| {
                let Some(d) = dw.upgrade() else { return };
                let dir = crate::pastas::downloads();
                let _ = std::fs::create_dir_all(&dir);
                let p = crate::widgets::nome_livre(&dir, tr("diagram"), fmt);
                let r = if fmt == "png" { d.save_png(&p) } else { d.save_vector(&p) };
                match r {
                    Ok(()) => {
                        b.set_tooltip_text(Some(&trf("Saved in {folder}", &[("folder", dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())])));
                        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                            eprintln!("debug diagrama: {fmt} salvo {}", p.display());
                        }
                    }
                    Err(e) => eprintln!("kalango-shell: diagrama em {fmt}: {e}"),
                }
            });
            crate::central::posicao_debug(&format!("canvas exportar {fmt}"), b.upcast_ref());
            ex.append(&b);
        }
        self.canvas_box.append(&ex);
        let inserir = gtk::Button::new();
        inserir.add_css_class("cfg-botao-acento");
        let ic = caixa(Orientation::Horizontal, 6, "");
        ic.set_halign(Align::Center);
        ic.append(&ui_icon("doc", "#A5A1E2", 14));
        let il = rotulo(tr("Insert in the open document"), "");
        ic.append(&il);
        inserir.set_child(Some(&ic));
        inserir.set_margin_top(10);
        let dw = Rc::downgrade(&d);
        inserir.connect_clicked(move |_| {
            let Some(d) = dw.upgrade() else { return };
            if !crate::editor::insert_in_open(&format!("```mermaid\n{}\n```", d.src.borrow().trim_end())) {
                il.set_text(tr("Open a document in the Editor first"));
            }
        });
        self.canvas_box.append(&inserir);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug central: canvas do diagrama ({} linhas)", d.src.borrow().lines().count());
        }
    }

    /// The Changes tab (design "Chat · código (plano, diff e terminal)"): the repository's branch and the files the
    /// conversation changed (+/−), the checks the agent ran (their exit), what changed, and a commit message the local
    /// model suggests — "Criar commit" commits those files (the person's click; "Desfazer" takes it back). Nothing is
    /// pushed from here.
    /// The tabs for this conversation: code mode (files changed in a git repository) shows Changes, Files and Terminal
    /// — Changes in front when it starts —; otherwise Context, Files and Canvas.
    fn set_mode(&self, codigo: bool, comandos: bool, web: bool) {
        let antes = self.abas[3].get_visible();
        let antes_web = self.abas[5].get_visible();
        let web = web && !codigo;
        self.abas[0].set_visible(!codigo);
        self.abas[1].set_visible(!web);
        self.abas[5].set_visible(web);
        // (the design's order: Browser, Canvas, Context — otherwise Context, Files, Canvas)
        if let Some(bx) = self.abas[2].parent().and_downcast::<gtk::Box>() {
            bx.reorder_child_after(&self.abas[2], Some(if web { &self.abas[5] } else { &self.abas[1] }));
        }
        self.abas[2].set_visible(!codigo || self.abas[2].is_sensitive());
        self.abas[3].set_visible(codigo);
        // (three tabs at most, as the design: a fourth would widen the panel past what a medium window holds)
        self.abas[4].set_visible(codigo);
        self.abas[4].set_sensitive(comandos);
        // (the tabs' own flags: is_visible is false while the panel is not on screen)
        let ativa_some = self.abas.iter().any(|a| a.is_active() && !a.get_visible());
        if codigo && (!antes || ativa_some) {
            self.abas[3].set_active(true);
        } else if web && (!antes_web || ativa_some) {
            self.abas[5].set_active(true);
        } else if ativa_some {
            self.abas[if codigo { 3 } else { 0 }].set_active(true);
        }
        self.ajustar_painel();
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            let nomes: Vec<String> = self.abas.iter().filter(|a| a.get_visible()).map(|a| a.label().unwrap_or_default().to_string()).collect();
            eprintln!("debug conversas: abas {}", nomes.join(" · "));
        }
    }

    /// The Terminal tab: every command the agent ran here, in order, as the conversation shows them.
    fn terminal_render(self: &Rc<Self>) {
        while let Some(x) = self.terminal_box.first_child() {
            self.terminal_box.remove(&x);
        }
        let ev = conversa::eventos(&self.conv.id());
        let mut args: Option<Value> = None;
        let mut n = 0;
        for e in &ev {
            if e["tipo"] == "ferramenta" && e["nome"] == "tarefas.executar" {
                args = Some(e["argumentos"].clone());
            }
            if e["tipo"] == "resultado" && e["nome"] == "tarefas.executar" {
                if let Some(a) = args.take() {
                    if let Some(b) = crate::terminal_block::from_result(&a, e["texto"].as_str().unwrap_or("")) {
                        self.terminal_box.append(&b);
                        n += 1;
                    }
                }
            }
        }
        // (one still running: its line, waiting)
        if let Some(a) = args {
            let prog = a["programa"].as_str().unwrap_or("");
            let lista: Vec<String> = a["argumentos"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
            let l = caixa(Orientation::Horizontal, 8, "conversas-ctx");
            let sp = gtk::Spinner::new();
            sp.start();
            l.append(&sp);
            let c = rotulo(&format!("$ {}", crate::terminal_block::command_line(prog, &lista)), "md-codigo");
            c.set_xalign(0.0);
            c.set_ellipsize(gtk::pango::EllipsizeMode::End);
            c.set_max_width_chars(1);
            c.set_hexpand(true);
            l.append(&c);
            self.terminal_box.append(&l);
        }
        if n == 0 && self.terminal_box.first_child().is_none() {
            let v = rotulo(tr("The agent has not run commands in this conversation."), "central-agente-sub");
            v.set_xalign(0.0);
            v.set_wrap(true);
            self.terminal_box.append(&v);
        }
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug conversas: terminal com {n} comandos");
        }
    }

    fn alteracoes_render(self: &Rc<Self>) {
        while let Some(x) = self.alteracoes_box.first_child() {
            self.alteracoes_box.remove(&x);
        }
        let Some((root, arquivos)) = self.repo.borrow().clone() else { return };
        let git = |args: &[&str]| -> String {
            std::process::Command::new("git").arg("-C").arg(&root).args(args).stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null()).output().ok().filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
        };
        let ramo = git(&["branch", "--show-current"]);
        // the files with their lines added and removed (new files: all added)
        let rel: Vec<String> = arquivos.iter().filter_map(|a| std::path::Path::new(a).strip_prefix(&root).ok().map(|p| p.display().to_string())).collect();
        let mut args = vec!["diff", "--numstat", "HEAD", "--"];
        args.extend(rel.iter().map(String::as_str));
        let num = git(&args);
        // ("added\tremoved\tpath" per file)
        let mut linhas: Vec<(String, u64, u64)> = num.lines().filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            (c.len() >= 3).then(|| (c[2].to_string(), c[0].parse().unwrap_or(0), c[1].parse().unwrap_or(0)))
        }).collect();
        let mut args = vec!["status", "--porcelain", "--"];
        args.extend(rel.iter().map(String::as_str));
        for l in git(&args).lines().filter(|l| l.starts_with("??")) {
            let f = l[3..].to_string();
            let n = std::fs::read_to_string(root.join(&f)).map(|t| t.lines().count() as u64).unwrap_or(0);
            if !linhas.iter().any(|x| x.0 == f) {
                linhas.push((f, n, 0));
            }
        }
        let secao = |t: &str, glifo: &str| {
            let l = caixa(Orientation::Horizontal, 8, "");
            l.set_margin_top(16);
            l.set_margin_bottom(10);
            if glifo == "spark" { l.append(&estrela(12, "#A5A1E2")) } else { l.append(&ui_icon(glifo, "#8E8F99", 13)) }
            l.append(&rotulo(t, "central-painel-secao"));
            l
        };
        let cab = caixa(Orientation::Horizontal, 8, "");
        cab.append(&ui_icon("ramo", "#A5A1E2", 16));
        let r = rotulo(if ramo.is_empty() { tr("no branch") } else { &ramo }, "alteracoes-ramo");
        r.set_hexpand(true);
        r.set_xalign(0.0);
        cab.append(&r);
        cab.append(&rotulo(&if linhas.len() == 1 { tr("1 file").to_string() } else { trf("{n} files", &[("n", linhas.len().to_string())]) }, "rotinas-chip"));
        self.alteracoes_box.append(&cab);
        let lista = caixa(Orientation::Vertical, 0, "central-cartao");
        lista.set_margin_top(10);
        if linhas.is_empty() {
            let v = rotulo(tr("No change left to commit."), "central-agente-sub");
            v.set_xalign(0.0);
            lista.append(&v);
        }
        // (a file picked shows its change under the list)
        let previa_diff = caixa(Orientation::Vertical, 0, "");
        previa_diff.set_margin_top(10);
        previa_diff.set_visible(false);
        let mut botoes: Vec<gtk::Button> = Vec::new();
        for (f, a, r) in &linhas {
            let b = gtk::Button::new();
            b.add_css_class("alteracoes-arquivo");
            let row = caixa(Orientation::Horizontal, 8, "");
            row.append(&ui_icon("doc", "#8E8F99", 14));
            let n = rotulo(f, "central-limite-nome");
            n.set_hexpand(true);
            n.set_xalign(0.0);
            n.set_ellipsize(gtk::pango::EllipsizeMode::Start);
            n.set_max_width_chars(1);
            row.append(&n);
            row.append(&rotulo(&format!("+{a}"), "alteracoes-mais"));
            row.append(&rotulo(&format!("−{r}"), "alteracoes-menos"));
            b.set_child(Some(&row));
            lista.append(&b);
            botoes.push(b);
        }
        let botoes = Rc::new(botoes);
        for (i, (f, _, _)) in linhas.iter().enumerate() {
            let (bs, pd, root2, f) = (botoes.clone(), previa_diff.clone(), root.clone(), f.clone());
            botoes[i].connect_clicked(move |b| {
                let ja = b.has_css_class("selecionado");
                for x in bs.iter() {
                    x.remove_css_class("selecionado");
                }
                while let Some(x) = pd.first_child() {
                    pd.remove(&x);
                }
                pd.set_visible(!ja);
                if ja {
                    return;
                }
                b.add_css_class("selecionado");
                // the version in the last commit (none for a new file) against the one on disk
                let antes = std::process::Command::new("git").arg("-C").arg(&root2).args(["show", &format!("HEAD:{f}")]).stderr(std::process::Stdio::null())
                    .output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
                let agora = std::fs::read_to_string(root2.join(&f)).unwrap_or_default();
                pd.append(&crate::diff_card::view(&f, &antes, &agora));
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug alteracoes: diff de {f}");
                }
            });
            crate::central::posicao_debug(&format!("alteracoes arquivo {i}"), botoes[i].upcast_ref());
        }
        self.alteracoes_box.append(&lista);
        self.alteracoes_box.append(&previa_diff);
        // the checks: the programs the agent ran in this conversation, with their exit
        let ev = conversa::eventos(&self.conv.id());
        // (command, Some((exit, summary)) once it ended — None while it runs)
        let mut cmds: Vec<(String, Option<(Option<i64>, String)>)> = Vec::new();
        for e in &ev {
            if e["tipo"] == "ferramenta" && matches!(e["nome"].as_str(), Some("tarefas.executar")) {
                let a = &e["argumentos"];
                let args: Vec<String> = a["argumentos"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
                cmds.push((crate::terminal_block::command_line(a["programa"].as_str().unwrap_or(""), &args), None));
            }
            if e["tipo"] == "resultado" && e["nome"] == "tarefas.executar" {
                if let Some(last) = cmds.last_mut().filter(|c| c.1.is_none()) {
                    let v = serde_json::from_str::<Value>(e["texto"].as_str().unwrap_or("")).unwrap_or(Value::Null);
                    let code = v["codigo"].as_i64();
                    let mut out = v["saida"].as_str().unwrap_or("").to_string();
                    out.push('\n');
                    out += v["erros"].as_str().unwrap_or("");
                    let resumo = crate::terminal_block::summary(&last.0, code, v["tempo_ms"].as_u64().unwrap_or(0), &out);
                    last.1 = Some((code, resumo));
                }
            }
        }
        if !cmds.is_empty() {
            self.alteracoes_box.append(&secao(tr("CHECKS"), "escudo"));
            let c = caixa(Orientation::Vertical, 0, "central-cartao");
            for (cmd, fim) in cmds.iter().rev().take(6) {
                let row = caixa(Orientation::Horizontal, 10, "alteracoes-verificacao");
                let col = caixa(Orientation::Vertical, 2, "");
                col.set_hexpand(true);
                let n = rotulo(cmd, "central-limite-nome");
                n.set_xalign(0.0);
                n.set_ellipsize(gtk::pango::EllipsizeMode::End);
                n.set_max_width_chars(1);
                col.append(&n);
                match fim {
                    Some((code, resumo)) => {
                        let ok = *code == Some(0);
                        row.append(&ui_icon(if ok { "check" } else { "erro" }, if ok { "#8ED2C3" } else { "#EFA2A6" }, 14));
                        let sub = rotulo(resumo, "central-agente-sub");
                        sub.set_xalign(0.0);
                        sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
                        sub.set_max_width_chars(1);
                        col.append(&sub);
                        row.append(&col);
                        let chip = rotulo(&code.map(|c| if c == 0 { "ok".to_string() } else { format!("exit {c}") }).unwrap_or_else(|| "—".into()), if ok { "alteracoes-ok" } else { "alteracoes-falhou" });
                        chip.set_valign(Align::Center);
                        row.append(&chip);
                    }
                    None => {
                        let sp = gtk::Spinner::new();
                        sp.start();
                        sp.set_size_request(14, 14);
                        row.append(&sp);
                        let sub = rotulo(tr("running"), "central-agente-sub");
                        sub.set_xalign(0.0);
                        col.append(&sub);
                        row.append(&col);
                    }
                }
                c.append(&row);
            }
            self.alteracoes_box.append(&c);
        }
        // what changed: the local model's summary of the diff when it comes; until then the agent's last answer, its
        // first whole sentences
        let mudou = rotulo("", "central-agente-sub");
        if let Some(t) = ev.iter().rev().find(|e| e["tipo"] == "resposta").and_then(|e| e["texto"].as_str()) {
            self.alteracoes_box.append(&secao(tr("WHAT CHANGED"), "spark"));
            mudou.set_text(&first_sentences(t, 220));
            let l = mudou.clone();
            l.set_wrap(true);
            l.set_xalign(0.0);
            let c = caixa(Orientation::Vertical, 0, "central-cartao");
            c.append(&l);
            self.alteracoes_box.append(&c);
        }
        // the commit, at the panel's bottom: the message the local model suggests (editable), the person's click commits
        let empurra = caixa(Orientation::Vertical, 0, "");
        empurra.set_vexpand(true);
        self.alteracoes_box.append(&empurra);
        self.alteracoes_box.append(&secao(tr("SUGGESTED MESSAGE"), "lapis"));
        let msg = gtk::Entry::new();
        msg.add_css_class("canvas-pedido");
        msg.set_placeholder_text(Some(tr("the local model writes it…")));
        self.alteracoes_box.append(&msg);
        let acoes = caixa(Orientation::Horizontal, 8, "");
        acoes.set_margin_top(10);
        acoes.set_homogeneous(true);
        let criar = gtk::Button::new();
        criar.add_css_class("botao-principal");
        let cc = caixa(Orientation::Horizontal, 6, "");
        cc.set_halign(Align::Center);
        cc.append(&ui_icon("check", "#1C1D24", 13));
        cc.append(&rotulo(tr("Create commit"), ""));
        criar.set_child(Some(&cc));
        criar.set_sensitive(!linhas.is_empty());
        let pr = gtk::Button::new();
        pr.add_css_class("midia-acao");
        let pc2 = caixa(Orientation::Horizontal, 6, "");
        pc2.set_halign(Align::Center);
        pc2.append(&ui_icon("compartilhar", "#C9CAD1", 13));
        pc2.append(&rotulo(tr("Open PR"), ""));
        pr.set_child(Some(&pc2));
        // "Abrir PR": the branch goes to the remote and the PR opens (gh) — only after the person confirms what
        // leaves, in two clicks; without a remote or gh it says what is missing
        let pr_estado = rotulo("", "central-agente-sub");
        pr_estado.set_xalign(0.0);
        pr_estado.set_wrap(true);
        pr_estado.set_visible(false);
        let confirmar_pr = Rc::new(Cell::new(false));
        let (root4, ramo4, w4, est) = (root.clone(), ramo.clone(), Rc::downgrade(self), pr_estado.clone());
        pr.connect_clicked(move |b| {
            let Some(c) = w4.upgrade() else { return };
            let falta = pr_missing(&root4, &ramo4, c.commit_feito.borrow().is_some());
            if let Some(f) = falta {
                est.set_text(&f);
                est.set_visible(true);
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug alteracoes: abrir PR — {f}");
                }
                return;
            }
            if !confirmar_pr.replace(true) {
                est.set_text(&trf("Send the branch {branch} to origin and open the pull request? Click again to confirm.", &[("branch", ramo4.clone())]));
                est.set_visible(true);
                return;
            }
            b.set_sensitive(false);
            est.set_text(tr("Sending the branch…"));
            let (root5, ramo5, est2, bw) = (root4.clone(), ramo4.clone(), est.clone(), b.downgrade());
            crate::lateral::em_segundo_plano(move || open_pr(&root5, &ramo5), move |r| {
                if let Some(b) = bw.upgrade() {
                    b.set_sensitive(true);
                }
                match r {
                    Ok(url) => est2.set_markup(&format!("{} <a href=\"{}\">{}</a>", glib::markup_escape_text(tr("Pull request opened:")), glib::markup_escape_text(&url), glib::markup_escape_text(&url))),
                    Err(e) => est2.set_text(&trf("The pull request did not open: {error}", &[("error", e)])),
                }
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug alteracoes: abrir PR → {}", est2.text());
                }
            });
        });
        crate::central::posicao_debug("alteracoes abrir pr", pr.upcast_ref());
        acoes.append(&criar);
        acoes.append(&pr);
        self.alteracoes_box.append(&acoes);
        self.alteracoes_box.append(&pr_estado);
        let notal = caixa(Orientation::Horizontal, 6, "");
        notal.append(&ui_icon("escudo", "#8ED2C3", 13));
        let nota = rotulo(tr("Push only with your approval · everything can be undone"), "central-agente-sub");
        nota.set_xalign(0.0);
        nota.set_hexpand(true);
        nota.set_wrap(true);
        notal.append(&nota);
        notal.set_margin_top(8);
        notal.set_margin_bottom(12);
        self.alteracoes_box.append(&notal);
        crate::central::posicao_debug("alteracoes criar commit", criar.upcast_ref());
        // the suggestion, off the interface thread (the diff of these files)
        if !linhas.is_empty() {
            let (root2, rel2, mw, rw) = (root.clone(), rel.clone(), msg.downgrade(), mudou.downgrade());
            crate::lateral::em_segundo_plano(move || -> Option<(String, String)> {
                let mut diff = std::process::Command::new("git").arg("-C").arg(&root2).args(["diff", "HEAD", "--"]).args(&rel2).output().ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
                // (new files: their content as added lines)
                for f in &rel2 {
                    if !diff.contains(&format!("b/{f}")) {
                        if let Ok(t) = std::fs::read_to_string(root2.join(f)) {
                            diff.push_str(&format!("+++ b/{f}\n{}\n", t.lines().take(80).map(|l| format!("+{l}")).collect::<Vec<_>>().join("\n")));
                        }
                    }
                }
                let mut c = std::process::Command::new(crate::voz::agent_binary()).arg("commit-mensagem").stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
                std::io::Write::write_all(c.stdin.as_mut()?, diff.as_bytes()).ok()?;
                drop(c.stdin.take());
                let o = c.wait_with_output().ok()?;
                let v: Value = serde_json::from_slice(&o.stdout).ok()?;
                v["mensagem"].as_str().map(|m| (m.to_string(), v["resumo"].as_str().unwrap_or("").trim().to_string()))
            }, move |r| {
                if let (Some(e), Some((m, resumo))) = (mw.upgrade(), r) {
                    if e.text().is_empty() {
                        e.set_text(&m);
                    }
                    if let Some(l) = rw.upgrade().filter(|_| !resumo.is_empty()) {
                        l.set_text(&resumo);
                    }
                    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                        eprintln!("debug alteracoes: mensagem sugerida «{m}»");
                    }
                }
            });
        }
        let (w, root3, rel3, mw) = (Rc::downgrade(self), root.clone(), rel.clone(), msg.downgrade());
        criar.connect_clicked(move |b| {
            let (Some(c), Some(m)) = (w.upgrade(), mw.upgrade()) else { return };
            let texto = m.text().trim().to_string();
            if texto.is_empty() {
                m.grab_focus();
                return;
            }
            let git = |args: &[&str]| std::process::Command::new("git").arg("-C").arg(&root3).args(args).stdin(std::process::Stdio::null()).output();
            let mut add = vec!["add", "--"];
            add.extend(rel3.iter().map(String::as_str));
            let ok = git(&add).is_ok_and(|o| o.status.success()) && git(&["commit", "-m", &texto, "--"]).is_ok_and(|o| o.status.success());
            let hash = git(&["rev-parse", "--short", "HEAD"]).ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
            if ok {
                *c.commit_feito.borrow_mut() = Some(hash.clone());
                b.set_child(Some(&rotulo(&trf("Committed {hash} · Undo", &[("hash", hash.clone())]), "")));
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug alteracoes: commit {hash} «{texto}»");
                }
                // (a second click undoes exactly that commit, keeping the changes)
                let (cw, root4) = (Rc::downgrade(&c), root3.clone());
                b.connect_clicked(move |b| {
                    let Some(c) = cw.upgrade() else { return };
                    let Some(h) = c.commit_feito.borrow_mut().take() else { return };
                    let cabeca = std::process::Command::new("git").arg("-C").arg(&root4).args(["rev-parse", "--short", "HEAD"]).output().ok()
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
                    if cabeca == h && std::process::Command::new("git").arg("-C").arg(&root4).args(["reset", "--soft", "HEAD~1"]).status().is_ok_and(|s| s.success()) {
                        b.set_child(Some(&rotulo(tr("Undone: the changes are back, not committed"), "")));
                        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                            eprintln!("debug alteracoes: commit {h} desfeito");
                        }
                    }
                });
            } else {
                b.set_child(Some(&rotulo(tr("The commit did not go through (git)"), "")));
            }
        });
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug alteracoes: {} em {} ({} arquivos, {} verificações)", ramo, root.display(), linhas.len(), cmds.len());
        }
    }

    /// The strip's tab of the context's open tab, marked.
    fn marcar_faixa(&self) {
        for (i, (glifo, b)) in self.faixa_botoes.iter().enumerate() {
            let ativa = self.abas.get(i).is_some_and(|a| a.is_active());
            if ativa { b.add_css_class("ativo") } else { b.remove_css_class("ativo") }
            if let Some(img) = b.child().and_downcast::<gtk::Image>() {
                conversa::set_icon(&img, glifo, if ativa { "#A5A1E2" } else { "#8E8F99" });
            }
        }
    }

    /// What the right panel shows, for its width rule: Canvas and Browser are wide (a page needs room — designs
    /// "Central · canvas largo" and "Chat · navegador do agente"), the other tabs are rows.
    fn panel_kind(&self) -> PanelKind {
        match self.painel_stack.visible_child_name().as_deref() {
            Some("canvas" | "navegador") => PanelKind::Wide,
            _ => PanelKind::Narrow,
        }
    }

    /// The width the person gave the tab on screen (Canvas and Browser keep their own; the rest share the context's).
    fn saved_panel_width(&self) -> &Cell<Option<i32>> {
        match self.painel_stack.visible_child_name().as_deref() {
            Some("canvas") => &self.canvas_width,
            Some("navegador") => &self.browser_width,
            _ => &self.largura_contexto,
        }
    }

    /// The right panel's width (`widths`): the person's for this tab, or its share of the window (26 % for the
    /// context's rows, 45 % for a page, up to 70 %); expanded, all but the conversation's narrow column. Never more
    /// than the conversation leaves. Called at every resize of the window or of a panel.
    fn ajustar_painel(&self) {
        let janela = self.largura.get();
        let lista = if self.lateral.is_visible() && self.faixa.get() != Faixa::Estreita { self.lateral.width_request() + HIT } else { 64 };
        let w = if self.expanded.get() {
            widths::expanded_panel_width(janela, lista, HIT)
        } else {
            widths::right_panel_width(self.panel_kind(), janela, lista, self.saved_panel_width().get(), HIT)
        };
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() && self.painel.is_visible() && self.painel.width_request() != w {
            eprintln!("debug central: larguras janela {janela} · esquerda {lista} · painel {w} ({:?}{}) · coluna {}", self.panel_kind(),
                if self.expanded.get() { ", expandido" } else { "" }, widths::reading_column(janela - lista - HIT - w));
        }
        if self.painel.width_request() != w {
            self.painel.set_size_request(w, -1);
        }
    }

    /// Expands the canvas over most of the window (`true`: the side bar gives way to the rail) or puts it back (the
    /// side bar as it was). Only with Canvas or Browser on screen.
    fn expand(&self, on: bool) {
        if on == self.expanded.get() || (on && self.panel_kind() != PanelKind::Wide) {
            return;
        }
        self.expanded.set(on);
        if on {
            self.list_closed_before.set(self.recolhida.get());
            self.recolhida.set(true);
        } else {
            self.recolhida.set(self.list_closed_before.get());
        }
        self.mark_expand_button();
        self.arrumar();
    }

    /// The expand button: only beside a page (Canvas, Browser) in a window with side panels; lit while expanded.
    fn mark_expand_button(&self) {
        let b = &self.expand_button;
        b.set_visible(self.faixa.get() != Faixa::Estreita && self.panel_kind() == PanelKind::Wide);
        let on = self.expanded.get();
        if on { b.add_css_class("expandido") } else { b.remove_css_class("expandido") }
        if let Some(img) = b.child().and_downcast::<gtk::Image>() {
            conversa::set_icon(&img, if on { "recolher" } else { "expandir" }, if on { "#A5A1E2" } else { "#8E8F99" });
        }
        b.set_tooltip_text(Some(if on { tr("Put the canvas back") } else { tr("Expand the canvas") }));
    }

    /// The side bar's width: the person's, or 22 % of the window.
    fn ajustar_lista(&self) {
        let janela = self.largura.get();
        let w = panel_width(janela, self.largura_lista.get(), 0.22, LISTA_PADRAO, LISTA_LIMITES, janela - self.painel_base - 2 * HIT - CONVERSA_MINIMA);
        if self.lateral.width_request() != w {
            self.lateral.set_size_request(w, -1);
        }
    }

    /// The edges of the context and of the side bar: dragging them sets the width the person keeps (the shell's
    /// settings), a double click goes back to the default.
    fn alcas(self: &Rc<Self>) -> (gtk::Widget, gtk::Widget) {
        let feitas = |contexto: bool| -> gtk::Widget {
            let (w1, w2, w3) = (Rc::downgrade(self), Rc::downgrade(self), Rc::downgrade(self));
            resize_handle::new(if contexto { -1 } else { 1 }, resize_handle::Panel {
                // (the width set, as the drag sets it: `width()` would leave out the padding)
                width: Box::new(move || w1.upgrade().map(|c| if contexto { c.painel.width_request() } else { c.lateral.width_request() }).unwrap_or(0)),
                resize: Box::new(move |w, guardar| {
                    let Some(c) = w2.upgrade() else { return };
                    if contexto {
                        // (the tab's own limits: a page may go up to 70 % of the window; dragging an expanded canvas
                        // makes the dragged width its own)
                        let lim = widths::panel_rule(c.panel_kind(), c.largura.get()).limits;
                        if c.expanded.replace(false) {
                            c.mark_expand_button();
                        }
                        c.saved_panel_width().set(Some(w.clamp(lim.0, lim.1)));
                    } else {
                        c.largura_lista.set(Some(w.clamp(LISTA_LIMITES.0, LISTA_LIMITES.1)));
                    }
                    c.ajustar_lista();
                    c.ajustar_painel();
                    if guardar {
                        c.guardar_larguras();
                    }
                }),
                reset: Box::new(move || {
                    let Some(c) = w3.upgrade() else { return };
                    if contexto { c.saved_panel_width().set(None) } else { c.largura_lista.set(None) }
                    c.ajustar_lista();
                    c.ajustar_painel();
                    c.guardar_larguras();
                }),
            })
            .upcast()
        };
        (feitas(true), feitas(false))
    }

    fn guardar_larguras(&self) {
        crate::configuracoes::salvar_ajuste("central_paineis", json!({"contexto": self.largura_contexto.get(), "lista": self.largura_lista.get(), "canvas": self.canvas_width.get(), "navegador": self.browser_width.get()}));
    }

    /// Puts the side bar, the context, its tabs and the header's buttons where the band and the person want them.
    fn arrumar(&self) {
        let f = self.faixa.get();
        let estreita = f == Faixa::Estreita;
        let pag = self.na_pagina.get();
        let em_conversa = self.vista.visible_child_name().as_deref() == Some("conversa");
        let lista = !self.recolhida.get();
        let painel = em_conversa && self.painel_aberto.get();
        self.arrumando.set(true);
        self.por_em_folha(&self.lateral, estreita, Align::Start, &self.casa_lateral);
        self.por_em_folha(&self.painel, estreita, Align::End, &self.casa_painel);
        self.lateral.set_visible(pag && lista);
        self.painel.set_visible(pag && painel);
        self.faixa_abas.set_visible(em_conversa && !painel && !estreita);
        self.marcar_faixa();
        self.veu.set_visible(estreita && pag && (lista || painel));
        self.controles_meio.set_visible(estreita || !painel);
        self.controles_painel.set_visible(!estreita);
        self.abrir_lista_b.set_visible(estreita);
        self.faisca.set_visible(estreita);
        self.subtitulo.set_visible(estreita && em_conversa);
        self.chip_agente_box.set_visible(!estreita && em_conversa);
        self.painel_b.set_visible(estreita && em_conversa);
        self.painel_b.set_active(painel);
        self.ajustar_lista();
        self.ajustar_painel();
        self.mark_expand_button();
        if let Some(a) = self.alca_contexto.borrow().as_ref() {
            a.set_visible(pag && painel && !estreita);
        }
        if let Some(a) = self.alca_lista.borrow().as_ref() {
            a.set_visible(pag && lista && !estreita);
        }
        let m = conversa::meta(&self.conv.id());
        self.fixar.set_visible(!estreita && em_conversa && m["titulo"].is_string());
        self.apagar.set_visible(!estreita && em_conversa && m["titulo"].is_string());
        self.arrumando.set(false);
        // the rail: on another page, or beside a closed side bar (not in a narrow window: one column)
        if let Some(r) = self.recolher.borrow().as_ref() {
            r(!pag || (!estreita && !lista), pag && !estreita && !lista);
        }
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() && pag {
            eprintln!("debug central: conversas em {f:?} ({} px) · contexto {painel} · lista {lista}", self.largura.get());
            for (i, a) in self.abas.iter().enumerate() {
                crate::central::posicao_debug(&format!("conversas aba {i}"), a.upcast_ref());
            }
            crate::central::posicao_debug("conversas expandir", self.expand_button.upcast_ref());
            // (what keeps the window from getting narrower: each part's minimum width)
            let min = |w: &gtk::Widget| if w.is_visible() { w.measure(Orientation::Horizontal, -1).0 } else { 0 };
            eprintln!("debug central: mínimos · lateral {} · conversa {} · painel {} · faixa {} · tudo {}", min(self.lateral.upcast_ref()), min(self.vista.upcast_ref()),
                min(self.painel.upcast_ref()), min(self.faixa_abas.upcast_ref()), min(self.raiz.upcast_ref()));
            // (the context's parts: what holds it wider)
            let mut partes: Vec<String> = Vec::new();
            let mut c = self.painel.first_child();
            while let Some(w) = c {
                partes.push(format!("{} {}", w.type_().name(), min(&w)));
                c = w.next_sibling();
            }
            let mut p = self.painel_stack.first_child();
            while let Some(w) = p {
                let nome = self.painel_stack.page(&w).name().map(|n| n.to_string()).unwrap_or_default();
                partes.push(format!("{nome} {}", w.measure(Orientation::Horizontal, -1).0));
                p = w.next_sibling();
            }
            eprintln!("debug central: painel · {}", partes.join(" · "));
        }
    }

    pub fn ao_recolher(&self, f: impl Fn(bool, bool) + 'static) {
        *self.recolher.borrow_mut() = Some(Box::new(f));
    }

    /// The page shows (from the rail or the side bar).
    pub fn mostrar(self: &Rc<Self>) {
        // the first time: where the person chose to start (Agents › Settings › General › Start on)
        if !self.ja_mostrou.replace(true) && agent_prefs_start_on_last() {
            if let Some((id, _)) = conversa::conversas().into_iter().next() {
                self.abrir(&id);
            }
        }
        if self.vista.visible_child_name().as_deref() != Some("conversa") {
            self.nova();
        }
        self.lista_render();
        self.atualizar(true);
        crate::central::posicao_debug("nova conversa botao", self.nova_b.upcast_ref());
    }

    fn desligadas_da_nova(&self) -> Vec<String> {
        self.nova_grupos.iter().filter(|(_, b)| !b.is_active()).flat_map(|(k, _)| GRUPOS.iter().find(|g| g.0 == *k).map(|g| g.3).unwrap_or(&[]).iter().map(|s| s.to_string())).collect()
    }

    /// A new conversation: the greeting, the composer, things to start from.
    pub fn nova(self: &Rc<Self>) {
        self.conv.nova();
        self.preenchendo.set(true);
        for (_, b) in &self.nova_grupos {
            b.set_active(true);
        }
        self.preenchendo.set(false);
        self.conv.set_desligadas(vec![]);
        self.nova_texto.buffer().set_text("");
        self.set_start_agent(None);
        let h = glib::DateTime::now_local().map(|d| d.hour()).unwrap_or(12);
        let nome = glib::real_name().to_string_lossy().split([',', ' ']).next().unwrap_or("").trim().to_string();
        let nome = if nome.is_empty() || nome == "Unknown" { glib::user_name().to_string_lossy().into_owned() } else { nome };
        let ola = if h < 12 { tr("Good morning, {name}.") } else if h < 18 { tr("Good afternoon, {name}.") } else { tr("Good evening, {name}.") };
        self.saudacao.set_text(&ola.replace("{name}", &nome));
        self.sugestoes_render();
        self.expand(false);
        self.vista.set_visible_child_name("nova");
        if self.faixa.get() == Faixa::Estreita {
            self.recolhida.set(true);
        }
        self.cabecalho();
        self.lista_render();
        let t = self.nova_texto.clone();
        glib::idle_add_local_once(move || {
            t.grab_focus();
        });
        crate::central::posicao_debug("nova conversa texto", self.nova_texto.upcast_ref());
    }

    /// A new conversation with something already written (the cursor at its end, for the person to go on).
    pub fn preencher(self: &Rc<Self>, t: &str) {
        self.nova();
        let b = self.nova_texto.buffer();
        b.set_text(t);
        b.place_cursor(&b.end_iter());
    }

    fn enviar_nova(self: &Rc<Self>) {
        let b = self.nova_texto.buffer();
        let t = b.text(&b.start_iter(), &b.end_iter(), false).trim().to_string();
        if t.is_empty() && self.nova_form.is_empty() {
            return;
        }
        self.comecar(&t);
    }

    /// Starts the conversation with this request (from the composer or a suggestion).
    pub fn comecar(self: &Rc<Self>, pedido: &str) {
        let d = self.desligadas_da_nova();
        self.conv.set_desligadas(d.clone());
        // the agent chosen on the start (and what its fields got)
        let start = self.start_agent.borrow().clone();
        let typed = (!self.nova_form.is_empty()).then(|| self.nova_form.value());
        if let Some(Err(e)) = &typed {
            // (checked here: nothing starts with a field missing or of the wrong kind)
            crate::fluxos_ui::show_error(&self.nova_erro, e);
            return;
        }
        crate::fluxos_ui::show_error(&self.nova_erro, "");
        self.conv.set_agent(start);
        if let Some(Ok(v)) = &typed {
            self.conv.input_form.fill(v);
        }
        self.vista.set_visible_child_name("conversa");
        self.conv.ask(pedido);
        let id = self.conv.id();
        let mut m = conversa::meta(&id);
        m["desligadas"] = json!(d);
        conversa::salvar_meta(&id, &m);
        self.cabecalho();
        self.lista_render();
        self.atualizar(true);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug central: conversa começou {id}");
        }
    }

    /// Opens a kept conversation (the side bar's own moves on to a new one: one engine per conversation).
    pub fn abrir(self: &Rc<Self>, id: &str) {
        if let Some(s) = self.shell.upgrade() {
            let lateral = s.right_bar().conversa.clone();
            if lateral.id() == id {
                lateral.nova();
            }
        }
        // (another conversation puts an expanded canvas back)
        self.expand(false);
        self.conv.abrir(id);
        let d: Vec<String> = conversa::meta(id)["desligadas"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
        self.conv.set_desligadas(d);
        self.vista.set_visible_child_name("conversa");
        if self.faixa.get() == Faixa::Estreita {
            self.recolhida.set(true);
        }
        self.cabecalho();
        self.lista_render();
        self.atualizar(true);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug central: conversa aberta {id} ({} eventos)", conversa::eventos(id).len());
        }
    }

    fn cabecalho(&self) {
        let em_conversa = self.vista.visible_child_name().as_deref() == Some("conversa");
        let m = conversa::meta(&self.conv.id());
        self.titulo.set_text(if em_conversa { m["titulo"].as_str().unwrap_or(tr("New conversation")) } else { tr("New conversation") });
        self.subtitulo.set_text(&format!("{} · {}", self.chip_agente.text(), tr("local")));
        self.arrumar();
        if m["fixada"].as_bool().unwrap_or(false) { self.fixar.add_css_class("ativo") } else { self.fixar.remove_css_class("ativo") }
    }

    // ---------------------------------------------------------------------------------------------- the person's agents

    /// What the picker leaves to the page: run a flow, create or manage agents; an agent chosen on the start.
    fn picked(self: &Rc<Self>, x: Picked) {
        match x {
            Picked::Agent(a) => self.set_start_agent(a),
            Picked::Run(a) => {
                let flow = a["config"]["flow_id"].as_str().unwrap_or("").to_string();
                if let Some(f) = self.ao_executar.borrow().as_ref() {
                    f(&flow);
                }
            }
            Picked::Create => {
                if let Some(f) = self.ir.borrow().as_ref() {
                    f("novo-agente");
                }
            }
            Picked::Manage => {
                if let Some(f) = self.ir.borrow().as_ref() {
                    f("agentes");
                }
            }
        }
    }

    /// Runs a flow when the picker's "Executar" is chosen (the agents center opens it live).
    pub fn ao_executar(&self, f: impl Fn(&str) + 'static) {
        *self.ao_executar.borrow_mut() = Some(Box::new(f));
    }

    /// A new conversation with one of the person's agents chosen ("Conversar" in Meus agentes).
    pub fn com_agente(self: &Rc<Self>, agent: Value) {
        self.nova();
        self.set_start_agent(Some(agent));
    }

    fn start_picker(self: &Rc<Self>) -> Rc<Picker> {
        if let Some(p) = self.start_picker.borrow().as_ref() {
            return p.clone();
        }
        let w = Rc::downgrade(self);
        let p = Picker::new(move |x| {
            if let Some(c) = w.upgrade() {
                c.picked(x);
                c.nova_texto.grab_focus();
            }
        });
        *self.start_picker.borrow_mut() = Some(p.clone());
        p
    }

    /// The new conversation's agent: its chip marked, its fields in the composer.
    fn set_start_agent(self: &Rc<Self>, agent: Option<Value>) {
        let agent = agent.filter(|a| !agent_picker::is_flow(a));
        let fields = agent.as_ref().map(|a| agent_picker::fields(&a["input_fields"])).unwrap_or_default();
        self.nova_form.set_fields(fields);
        crate::fluxos_ui::show_error(&self.nova_erro, "");
        self.nova_ph.set_text(if agent.is_some() { tr("Ask the agent · @ calls another") } else { tr("Ask anything · / commands · @ calls an agent") });
        *self.start_agent.borrow_mut() = agent;
        self.render_start_chips();
    }

    /// The start's chips: the system's agent, the three used last, "Todos os agentes" (the picker).
    fn render_start_chips(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        agent_picker::load_agents(move |r| {
            let Some(c) = w.upgrade() else { return };
            let agents = r.unwrap_or_default();
            crate::fluxos_ui::clear_box(&c.start_chips);
            let chosen = c.start_agent.borrow().clone();
            let chosen_id = chosen.as_ref().and_then(|a| a["id"].as_str().map(str::to_string));
            // the three used last (flows are run from the picker); a chosen one not among them takes the third place
            let mut shown: Vec<Option<Value>> = vec![None];
            shown.extend(agents.iter().filter(|a| !agent_picker::is_flow(a)).take(3).cloned().map(Some));
            if let Some(a) = chosen.as_ref().filter(|a| !shown.iter().flatten().any(|s| s["id"] == a["id"])) {
                if shown.len() >= 4 {
                    shown.pop();
                }
                shown.push(Some(a.clone()));
            }
            for a in shown {
                let b = gtk::Button::new();
                b.add_css_class("conversas-agente-chip");
                let ativo = a.as_ref().and_then(|a| a["id"].as_str()) == chosen_id.as_deref();
                if ativo {
                    b.add_css_class("ativo");
                }
                let cc = caixa(Orientation::Horizontal, 7, "");
                cc.append(&agent_picker::tile_for(a.as_ref(), 18));
                let name = a.as_ref().and_then(|a| a["name"].as_str().map(str::to_string)).unwrap_or_else(|| agent_picker::SYSTEM_AGENT.to_string());
                let l = rotulo(&name, "");
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                cc.append(&l);
                b.set_child(Some(&cc));
                crate::central::posicao_debug(&format!("conversas chip {name}"), b.upcast_ref());
                let w = Rc::downgrade(&c);
                b.connect_clicked(move |_| {
                    if let Some(c) = w.upgrade() {
                        c.set_start_agent(a.clone());
                        c.nova_texto.grab_focus();
                    }
                });
                c.start_chips.append(&b);
            }
            let all = gtk::Button::new();
            all.add_css_class("conversas-agente-chip");
            let cc = caixa(Orientation::Horizontal, 6, "");
            cc.append(&ui_icon("mais", "#8E8F99", 12));
            cc.append(&rotulo(tr("All agents"), ""));
            all.set_child(Some(&cc));
            all.set_tooltip_text(Some(tr("Your agents, a new one, or manage them")));
            crate::central::posicao_debug_sempre("conversas todos os agentes", all.upcast_ref());
            let w = Rc::downgrade(&c);
            all.connect_clicked(move |b| {
                if let Some(c) = w.upgrade() {
                    let cur = c.start_agent.borrow().as_ref().and_then(|a| a["id"].as_str().map(str::to_string));
                    c.start_picker().open(b, cur.as_deref(), "", true);
                }
            });
            c.start_chips.append(&all);
        });
    }

    /// The header's chip: the conversation's agent with its icon.
    fn show_header_agent(&self, agent: Option<&Value>) {
        crate::fluxos_ui::clear_box(&self.chip_tile);
        self.chip_tile.append(&agent_picker::tile_for(agent, 16));
        self.chip_agente.set_text(&agent.and_then(|a| a["name"].as_str().map(str::to_string)).unwrap_or_else(|| agent_picker::SYSTEM_AGENT.to_string()));
        self.subtitulo.set_text(&format!("{} · {}", self.chip_agente.text(), tr("local")));
    }

    /// The kept conversations in the side bar: pinned, today, yesterday, earlier (the search filters them).
    fn lista_render(self: &Rc<Self>) {
        while let Some(x) = self.lista.first_child() {
            self.lista.remove(&x);
        }
        let busca = self.busca.text().trim().to_lowercase();
        let todas: Vec<(String, Value)> = conversa::conversas().into_iter()
            .filter(|(_, m)| busca.is_empty() || m["titulo"].as_str().unwrap_or("").to_lowercase().contains(&busca)).collect();
        let hoje = inicio_do_dia();
        let atual = self.conv.id();
        let secoes: [(&str, Filtro); 4] = [
            (tr("PINNED"), Box::new(|m: &Value| m["fixada"].as_bool().unwrap_or(false))),
            (tr("TODAY"), Box::new(move |m: &Value| !m["fixada"].as_bool().unwrap_or(false) && m["atualizado"].as_i64().unwrap_or(0) >= hoje)),
            (tr("YESTERDAY"), Box::new(move |m: &Value| !m["fixada"].as_bool().unwrap_or(false) && (hoje - 86_400..hoje).contains(&m["atualizado"].as_i64().unwrap_or(0)))),
            (tr("EARLIER"), Box::new(move |m: &Value| !m["fixada"].as_bool().unwrap_or(false) && m["atualizado"].as_i64().unwrap_or(0) < hoje - 86_400)),
        ];
        for (nome, f) in secoes.iter() {
            let itens: Vec<&(String, Value)> = todas.iter().filter(|(_, m)| f(m)).collect();
            if itens.is_empty() {
                continue;
            }
            let s = rotulo(nome, "central-painel-secao");
            s.set_xalign(0.0);
            s.set_margin_top(12);
            s.set_margin_bottom(4);
            s.set_margin_start(10);
            self.lista.append(&s);
            for (id, m) in itens {
                let b = gtk::Button::new();
                b.add_css_class("conversas-item");
                if *id == atual && self.vista.visible_child_name().as_deref() == Some("conversa") {
                    b.add_css_class("ativo");
                }
                let c = caixa(Orientation::Vertical, 1, "");
                let t = rotulo(m["titulo"].as_str().unwrap_or(""), "conversas-item-titulo");
                t.set_xalign(0.0);
                t.set_ellipsize(gtk::pango::EllipsizeMode::End);
                t.set_max_width_chars(1);
                t.set_hexpand(true);
                c.append(&t);
                let quem = agent_picker::history_label(m).unwrap_or_else(|| tr("Agent").to_string());
                let sub = rotulo(&format!("{quem} · {}", quando(m["atualizado"].as_i64().unwrap_or(0))), "central-agente-sub");
                sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
                sub.set_max_width_chars(1);
                sub.set_xalign(0.0);
                c.append(&sub);
                b.set_child(Some(&c));
                let (w, id2) = (Rc::downgrade(self), id.clone());
                b.connect_clicked(move |_| {
                    if let Some(c) = w.upgrade() {
                        c.abrir(&id2);
                    }
                });
                crate::central::posicao_debug(&format!("conversa {id}"), b.upcast_ref());
                self.lista.append(&b);
            }
        }
        if todas.is_empty() {
            let v = rotulo(if busca.is_empty() { tr("No conversation yet.") } else { tr("No conversation matches.") }, "central-agente-sub");
            v.set_margin_top(14);
            v.set_margin_start(10);
            v.set_xalign(0.0);
            self.lista.append(&v);
        }
        // the person's tokens today
        let mut u = crate::widgets::ia::Uso::default();
        u.atualizar();
        self.rodape_tokens.set_text(&trf("{used} of {budget} tokens today", &[("used", mil(u.local + u.nuvem)), ("budget", mil(crate::widgets::ia::orcamento()))]));
    }

    /// Things to start from — each does something real: go on with the last conversation, the notices of today,
    /// analyse a file, organise a folder, a routine, what the agents did today.
    fn sugestoes_render(self: &Rc<Self>) {
        while let Some(x) = self.sugestoes.first_child() {
            self.sugestoes.remove(&x);
        }
        let ultima = conversa::conversas().into_iter().next();
        let avisos = self.shell.upgrade().map(|s| s.notificacoes().historico.borrow().iter().filter(|g| g.quando >= inicio_do_dia()).count()).unwrap_or(0);
        let mut cartoes: Vec<(&str, &str, String, String, &str)> = Vec::new();
        if let Some((id, m)) = &ultima {
            cartoes.push(("terminal", "teal", trf("Go on: {title}", &[("title", m["titulo"].as_str().unwrap_or("").to_string())]),
                trf("from where we stopped at {time}", &[("time", quando(m["atualizado"].as_i64().unwrap_or(0)))]), "continuar"));
            let _ = id;
        }
        cartoes.push(("sino", "accent", tr("Summarize what came in today").to_string(),
            if avisos == 1 { tr("1 notification").to_string() } else { trf("{n} notifications", &[("n", avisos.to_string())]) }, "avisos"));
        cartoes.push(("doc", "gold", tr("Analyse a file").to_string(), tr("text, PDF, a spreadsheet in CSV").to_string(), "arquivo"));
        cartoes.push(("pasta", "blue", tr("Organise a folder").to_string(), tr("Downloads by kind, with undo").to_string(), "organizar"));
        cartoes.push(("raio", "accent", tr("Automate a routine").to_string(), tr("the agent runs it by itself").to_string(), "rotina"));
        cartoes.push(("relogio", "teal", tr("What the agents did today").to_string(), tr("each action, with undo").to_string(), "linha"));
        for (i, (glifo, cor, titulo, sub, acao)) in cartoes.into_iter().take(6).enumerate() {
            let b = gtk::Button::new();
            b.add_css_class("conversas-sugestao");
            let c = caixa(Orientation::Vertical, 6, "");
            let ic = caixa(Orientation::Horizontal, 0, "conversas-sugestao-icone");
            ic.add_css_class(cor);
            ic.set_size_request(30, 30);
            ic.set_hexpand(false);
            ic.set_halign(Align::Start);
            let g = ui_icon(glifo, "#E2E2E6", 15);
            g.set_hexpand(true);
            ic.append(&g);
            c.append(&ic);
            let t = rotulo(&titulo, "rotinas-sugestao-titulo");
            t.set_xalign(0.0);
            t.set_ellipsize(gtk::pango::EllipsizeMode::End);
            t.set_max_width_chars(1);
            t.set_hexpand(true);
            t.set_margin_top(6);
            c.append(&t);
            let s = rotulo(&sub, "central-agente-sub");
            s.set_xalign(0.0);
            c.append(&s);
            b.set_child(Some(&c));
            let w = Rc::downgrade(self);
            let ultima_id = ultima.as_ref().map(|(id, _)| id.clone());
            b.connect_clicked(move |_| {
                let Some(c) = w.upgrade() else { return };
                match acao {
                    "continuar" => {
                        if let Some(id) = &ultima_id {
                            c.abrir(id);
                        }
                    }
                    "avisos" => {
                        if let Some(s) = c.shell.upgrade() {
                            s.right_bar().abrir(crate::lateral::Modo::Avisos);
                        }
                    }
                    "arquivo" => {
                        let w = Rc::downgrade(&c);
                        crate::arquivos::seletor::abrir_arquivo(tr("Agents"), tr("Analyse a file"), None, Vec::new(), None, move |p| {
                            if let Some(c) = w.upgrade() {
                                c.comecar(&trf("Read the file {path} and tell me what matters in it.", &[("path", p.display().to_string())]));
                            }
                        });
                    }
                    "organizar" => c.comecar(tr("Organise the Downloads folder into folders by kind of file.")),
                    "rotina" => {
                        if let Some(f) = c.ir.borrow().as_ref() {
                            f("rotinas");
                        }
                    }
                    _ => {
                        if let Some(f) = c.ir.borrow().as_ref() {
                            f("linha");
                        }
                    }
                }
            });
            b.set_size_request(220, -1);
            self.sugestoes.insert(&b, -1);
            let _ = i;
        }
    }

    /// The context panel from the conversation's events (again when they changed, or `forcar`).
    /// The Files tab (design "Chat · dados"): the conversation's files — each with its kind, size and lines or
    /// pages; a click shows its preview below — and the note that it was read here.
    fn arquivos_render(self: &Rc<Self>, todos: &[(String, bool)]) {
        let rotulo_secao = |t: &str| {
            let l = caixa(Orientation::Horizontal, 8, "");
            l.set_margin_bottom(10);
            l.append(&ui_icon("clipe", "#8E8F99", 13));
            l.append(&rotulo(t, "central-painel-secao"));
            l
        };
        self.arquivos.append(&rotulo_secao(tr("FILES OF THIS CONVERSATION")));
        if todos.is_empty() {
            let v = rotulo(tr("No file yet: add one with + in the composer, or the agent writes one."), "central-agente-sub");
            v.set_xalign(0.0);
            v.set_wrap(true);
            self.arquivos.append(&v);
            return;
        }
        // the pictures as a grid first (design "Chat · mídia": the media of this conversation)
        let imagens: Vec<&String> = todos.iter().map(|(t, _)| t).filter(|t| crate::media_block::is_image(std::path::Path::new(t))).collect();
        if !imagens.is_empty() {
            let g = gtk::FlowBox::new();
            g.set_selection_mode(gtk::SelectionMode::None);
            g.set_min_children_per_line(3);
            g.set_max_children_per_line(3);
            g.set_homogeneous(true);
            g.set_column_spacing(8);
            g.set_row_spacing(8);
            g.set_margin_bottom(12);
            for t in imagens.iter().take(9) {
                let pic = gtk::Picture::for_filename(t.as_str());
                pic.set_content_fit(gtk::ContentFit::Cover);
                pic.set_can_shrink(true);
                pic.set_size_request(-1, 88);
                pic.add_css_class("conversas-miniatura");
                g.insert(&pic, -1);
            }
            self.arquivos.append(&g);
        }
        let sel = self.previa.borrow().clone().filter(|p| todos.iter().any(|(t, _)| t == p)).unwrap_or_else(|| todos[0].0.clone());
        for (p, gerado) in todos {
            let path = std::path::PathBuf::from(p);
            let b = gtk::Button::new();
            b.add_css_class("conversas-arquivo");
            if *p == sel {
                b.add_css_class("ativo");
            }
            let r = caixa(Orientation::Horizontal, 10, "");
            let ext = path.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_else(|| "—".into());
            let tile = caixa(Orientation::Horizontal, 0, "conversas-arquivo-tipo");
            tile.add_css_class(match ext.as_str() { "CSV" | "TSV" | "XLSX" | "ODS" => "verde", "PDF" => "coral", "PNG" | "JPG" | "JPEG" | "WEBP" => "azul", _ => "roxo" });
            tile.set_size_request(32, 32);
            tile.set_hexpand(false);
            tile.set_valign(Align::Center);
            let tl = rotulo(&ext.chars().take(4).collect::<String>(), "conversas-arquivo-ext");
            tl.set_hexpand(true);
            tile.append(&tl);
            r.append(&tile);
            let t = caixa(Orientation::Vertical, 2, "");
            t.set_hexpand(true);
            let n = rotulo(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), "central-limite-nome");
            n.set_xalign(0.0);
            n.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            n.set_max_width_chars(1);
            t.append(&n);
            let d = rotulo(&file_detail(&path, *gerado), "central-agente-sub");
            d.set_xalign(0.0);
            d.set_ellipsize(gtk::pango::EllipsizeMode::End);
            d.set_max_width_chars(1);
            t.append(&d);
            r.append(&t);
            if *gerado {
                let abrir = gtk::Button::new();
                abrir.add_css_class("central-controle");
                abrir.set_child(Some(&ui_icon("download", "#8E8F99", 15)));
                abrir.set_tooltip_text(Some(tr("Open")));
                abrir.set_valign(Align::Center);
                let p2 = path.clone();
                abrir.connect_clicked(move |b| {
                    let l = gtk::FileLauncher::new(Some(&gio::File::for_path(&p2)));
                    l.launch(b.root().and_downcast::<gtk::Window>().as_ref(), None::<&gio::Cancellable>, |_| {});
                });
                r.append(&abrir);
            }
            b.set_child(Some(&r));
            let (w, pp) = (Rc::downgrade(self), p.clone());
            b.connect_clicked(move |_| {
                if let Some(c) = w.upgrade() {
                    *c.previa.borrow_mut() = Some(pp.clone());
                    c.atualizar(true);
                }
            });
            crate::central::posicao_debug(&format!("conversas arquivo {}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()), b.upcast_ref());
            self.arquivos.append(&b);
        }
        // the preview of the chosen one
        let path = std::path::PathBuf::from(&sel);
        let nome = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let sec = rotulo_secao(&format!("{} · {}", tr("PREVIEW"), nome.to_uppercase()));
        sec.set_margin_top(14);
        self.arquivos.append(&sec);
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif") {
            let pic = gtk::Picture::for_filename(&path);
            pic.set_can_shrink(true);
            pic.set_content_fit(gtk::ContentFit::Contain);
            pic.set_size_request(-1, 160);
            pic.add_css_class("conversas-previa");
            self.arquivos.append(&pic);
        } else {
            let texto = preview_text(&path).unwrap_or_else(|| tr("No preview for this kind of file.").to_string());
            let l = rotulo(&texto, "conversas-previa-texto");
            l.set_xalign(0.0);
            l.set_selectable(true);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.set_max_width_chars(1);
            let caixa_p = caixa(Orientation::Vertical, 0, "conversas-previa");
            caixa_p.append(&l);
            self.arquivos.append(&caixa_p);
        }
        let nota = caixa(Orientation::Horizontal, 8, "conversas-nota-local");
        nota.set_margin_top(14);
        let ic = ui_icon("escudo", "#8ED2C3", 14);
        ic.set_valign(Align::Start);
        nota.append(&ic);
        let nt = rotulo(tr("Read by the local model: the file did not leave the computer."), "central-agente-sub");
        nt.set_xalign(0.0);
        nt.set_wrap(true);
        nota.append(&nt);
        self.arquivos.append(&nota);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug central: arquivos da conversa ({} · prévia {nome})", todos.len());
        }
    }

    pub fn atualizar(self: &Rc<Self>, forcar: bool) {
        let id = self.conv.id();
        let f = conversa::conversas_dir().join(&id).join("eventos.jsonl");
        let marca = std::fs::metadata(&f).map(|m| (m.len(), m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0))).unwrap_or((0, 0));
        if !forcar && marca == self.marca.get() {
            return;
        }
        self.marca.set(marca);
        let ev = conversa::eventos(&id);
        for c in [&self.contexto, &self.ferramentas, &self.memoria, &self.arquivos] {
            while let Some(x) = c.first_child() {
                c.remove(&x);
            }
        }
        let home = glib::home_dir().display().to_string();
        let curto = |p: &str| p.replacen(&home, "~", 1);
        // what it read and touched
        let mut itens: Vec<(String, String, String)> = Vec::new();
        let mut tocados: Vec<String> = Vec::new();
        for e in ev.iter().filter(|e| e["tipo"] == "ferramenta") {
            let nome = e["nome"].as_str().unwrap_or("");
            let a = &e["argumentos"];
            let desc = e["descricao"].as_str().unwrap_or(nome).to_string();
            let item = if let Some(u) = a["url"].as_str() {
                Some(("globo", u.split('/').nth(2).unwrap_or(u).to_string(), desc))
            } else if let Some(p) = a["caminho"].as_str().or(a["de"].as_str()) {
                Some((if nome == "arquivos.listar" { "pasta" } else { "doc" }, curto(p), desc))
            } else if let Some(p) = a["programa"].as_str() {
                Some(("terminal", p.to_string(), desc))
            } else if !nome.starts_with("memoria.") {
                // a reading of the system (disks, processes, services…)
                Some(("cpu", desc, tr("read from the system").to_string()))
            } else {
                None
            };
            if let Some((g, l, d)) = item {
                if !itens.iter().any(|(_, x, _)| *x == l) {
                    itens.push((g.to_string(), l, d));
                }
            }
            if matches!(nome, "arquivos.escrever" | "arquivos.mover") {
                if let Some(p) = a["para"].as_str().or(a["caminho"].as_str()) {
                    if !tocados.contains(&p.to_string()) {
                        tocados.push(p.to_string());
                    }
                }
            }
        }
        // a page the agent wrote (HTML): open on the canvas, the side bar folded to its rail (design "Chat · canvas com
        // HTML ao vivo")
        let pagina = tocados.iter().rev().map(|p| std::path::PathBuf::from(p.strip_prefix("~/").map(|r| glib::home_dir().join(r).display().to_string()).unwrap_or_else(|| p.clone())))
            .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm")) && p.is_file())
            // (or a page written in the answer itself, a ```html block: kept beside the conversation to be drawn)
            .or_else(|| {
                let md = ev.iter().rev().find(|e| e["tipo"] == "resposta").and_then(|e| e["texto"].as_str())?;
                let page = crate::html_canvas::page_in_answer(md)?;
                let p = conversa::conversas_dir().join(&id).join("pagina.html");
                if std::fs::read_to_string(&p).ok().as_deref() != Some(page.as_str()) {
                    std::fs::write(&p, &page).ok()?;
                }
                Some(p)
            });
        if let Some(p) = pagina {
            let nova = self.html.file().as_deref() != Some(p.as_path());
            self.html.open(&p);
            if nova {
                self.abas[2].set_sensitive(true);
                self.abas[2].set_visible(true);
                self.abas[2].set_active(true);
                self.uso.set_visible(false);
                if self.faixa.get() == Faixa::Larga && !self.recolhida.get() {
                    self.recolhida.set(true);
                    self.arrumar();
                }
                self.abrir_painel(true);
                self.canvas_render();
                self.ajustar_painel();
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug conversas: canvas com {}", p.display());
                }
            }
        } else if self.html.file().is_some() {
            // (another conversation: its own canvas, not the page of the one before)
            self.html.clear();
            if self.abas[2].is_active() {
                self.abas[0].set_active(true);
            }
            self.canvas_render();
            self.ajustar_painel();
        }
        for (g, l, d) in itens.iter().rev().take(8) {
            let r = caixa(Orientation::Horizontal, 10, "conversas-ctx");
            r.append(&ui_icon(g, "#8E8F99", 15));
            let c = caixa(Orientation::Vertical, 1, "");
            c.set_hexpand(true);
            let a = rotulo(l, "central-limite-nome");
            a.set_xalign(0.0);
            a.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            a.set_max_width_chars(1);
            a.set_hexpand(true);
            c.append(&a);
            let b = rotulo(d, "central-agente-sub");
            b.set_xalign(0.0);
            b.set_ellipsize(gtk::pango::EllipsizeMode::End);
            b.set_max_width_chars(1);
            c.append(&b);
            r.append(&c);
            self.contexto.append(&r);
        }
        if itens.is_empty() {
            let v = rotulo(tr("Nothing read yet."), "central-agente-sub");
            v.set_xalign(0.0);
            v.set_margin_top(8);
            v.set_margin_bottom(8);
            self.contexto.append(&v);
        }
        // the tools this conversation may use (on top of the permissions)
        let desligadas = self.conv.desligadas();
        let mut grupos: Vec<(String, String, String, Vec<String>)> = GRUPOS.iter().map(|(k, n, g, d)| (k.to_string(), tr(n).to_string(), g.to_string(), d.iter().map(|s| s.to_string()).collect())).collect();
        for e in std::fs::read_dir(crate::central_ajustes::pasta()).into_iter().flatten().filter_map(|e| e.ok()) {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "toml") {
                let n = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                grupos.push((format!("conector:{n}"), format!("{n} · MCP"), "cabo".into(), vec![format!("conector:{n}")]));
            }
        }
        self.preenchendo.set(true);
        for (_, nome, glifo, chaves) in grupos {
            let r = caixa(Orientation::Horizontal, 12, "memoria-chave");
            r.append(&ui_icon(&glifo, "#8E8F99", 15));
            let a = rotulo(&nome, "central-limite-nome");
            a.set_hexpand(true);
            a.set_xalign(0.0);
            // (a connector's long name gives way: the panel keeps its width)
            a.set_ellipsize(gtk::pango::EllipsizeMode::End);
            a.set_max_width_chars(1);
            a.set_tooltip_text(Some(&nome));
            r.append(&a);
            let sw = gtk::Switch::new();
            sw.add_css_class("central-switch");
            sw.set_valign(Align::Center);
            sw.set_active(!chaves.iter().all(|c| desligadas.contains(c)));
            let w = Rc::downgrade(self);
            sw.connect_state_set(move |_, on| {
                if let Some(c) = w.upgrade().filter(|c| !c.preenchendo.get()) {
                    let mut d = c.conv.desligadas();
                    d.retain(|x| !chaves.contains(x));
                    if !on {
                        d.extend(chaves.iter().cloned());
                    }
                    c.conv.set_desligadas(d.clone());
                    let id = c.conv.id();
                    let mut m = conversa::meta(&id);
                    if m["titulo"].is_string() {
                        m["desligadas"] = json!(d);
                        conversa::salvar_meta(&id, &m);
                    }
                }
                glib::Propagation::Proceed
            });
            r.append(&sw);
            self.ferramentas.append(&r);
        }
        self.preenchendo.set(false);
        // the memories the last request saw
        let mems: Vec<String> = ev.iter().rev().find(|e| e["tipo"] == "memoria").and_then(|e| e["itens"].as_array().cloned()).unwrap_or_default()
            .iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
        if mems.is_empty() {
            let v = rotulo(tr("No memory was used."), "central-agente-sub");
            v.set_xalign(0.0);
            self.memoria.append(&v);
        }
        for m in mems.iter().take(6) {
            let l = rotulo(&format!("“{m}”"), "central-limite-nome");
            l.set_xalign(0.0);
            l.set_wrap(true);
            self.memoria.append(&l);
        }
        // the files of this conversation: the ones the person added and the ones the agent wrote, with a preview
        let mut anexados: Vec<String> = Vec::new();
        for e in ev.iter().filter(|e| e["tipo"] == "pessoa") {
            for a in e["anexos"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                if !anexados.iter().any(|x| x == a) {
                    anexados.push(a.to_string());
                }
            }
        }
        let todos: Vec<(String, bool)> = anexados.iter().map(|a| (a.clone(), false)).chain(tocados.iter().filter(|t| !anexados.contains(t)).map(|t| (t.clone(), true))).collect();
        self.arquivos_render(&todos);
        // the repository the written files are in: the Changes tab opens
        let repo = tocados.iter().find_map(|t| git_root(std::path::Path::new(t))).map(|r| {
            let dentro: Vec<String> = tocados.iter().filter(|t| std::path::Path::new(t).starts_with(&r)).cloned().collect();
            (r, dentro)
        });
        let codigo = repo.is_some();
        *self.repo.borrow_mut() = repo;
        let comandos = ev.iter().any(|e| e["tipo"] == "ferramenta" && e["nome"] == "tarefas.executar");
        // the pages the agent read: the Browser tab, fed with them
        let web = ev.iter().any(|e| e["tipo"] == "resultado" && e["ok"] == true && e["nome"].as_str().is_some_and(|n| n.starts_with("web.")));
        self.set_mode(codigo, comandos, web);
        if web {
            self.navegador.feed(&id, &ev, self.conv.is_busy());
        }
        if self.abas[3].is_active() {
            self.alteracoes_render();
        }
        if self.abas[4].is_active() {
            self.terminal_render();
        }
        // tokens and the model
        let tokens = ev.iter().rev().find(|e| e["tipo"] == "uso").and_then(|e| e["tokens"].as_u64()).unwrap_or(0);
        self.tokens_v.set_text(&trf("{n} tokens", &[("n", mil(tokens))]));
        self.tokens_barra.set_value((tokens as f64 / 262_144.0).min(1.0));
        self.modelo_v.set_text(&format!("{} · {}", conversa::guess_model(), tr("local")));
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug central: conversa {id} ({} eventos, {} no contexto, {} lembranças)", ev.len(), itens.len(), mems.len());
        }
    }
}

/// The agents center starts on the last conversation (Agents › Settings › General › Start on).
/// A file's size and what it holds: "2,1 MB · 18 mil linhas", "189 KB · 2 páginas", "gerado agora".
fn file_detail(p: &std::path::Path, gerado: bool) -> String {
    let Ok(m) = std::fs::metadata(p) else { return tr("no longer there").to_string() };
    let size = glib::format_size(m.len()).to_string();
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let what = if ext == "pdf" {
        poppler::Document::from_file(&gio::File::for_path(p).uri(), None).ok().map(|d| trf("{n} pages", &[("n", d.n_pages().to_string())]))
    } else if m.len() < 50_000_000 && matches!(ext.as_str(), "csv" | "tsv" | "txt" | "md" | "json" | "log" | "rs" | "py" | "toml") {
        std::fs::read(p).ok().map(|b| {
            let n = b.iter().filter(|c| **c == b'\n').count();
            if n >= 1000 { trf("{n} thousand lines", &[("n", (n / 1000).to_string())]) } else { trf("{n} lines", &[("n", n.to_string())]) }
        })
    } else {
        None
    };
    let mut parts = vec![size];
    if gerado {
        parts.insert(0, tr("written by the agent").to_string());
    }
    parts.extend(what);
    parts.join(" · ")
}

/// A text file's first lines (a CSV's header and rows), for the Files tab's preview.
fn preview_text(p: &std::path::Path) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let f = std::fs::File::open(p).ok()?;
    let mut out = Vec::new();
    for l in BufReader::new(f).lines().take(9) {
        let l = l.ok()?;
        out.push(l.chars().take(80).collect::<String>());
    }
    if out.is_empty() {
        return None;
    }
    if out.len() == 9 {
        out[8] = "…".into();
    }
    Some(out.join("\n"))
}

/// The git repository a path is in (its top folder), when it is in one.
/// What "Abrir PR" still needs: a branch, a commit, a remote, gh — None when it can go.
fn pr_missing(root: &std::path::Path, ramo: &str, commit_feito: bool) -> Option<String> {
    let git = |args: &[&str]| std::process::Command::new("git").arg("-C").arg(root).args(args).stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null()).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    if ramo.is_empty() {
        return Some(tr("No branch: check one out first.").into());
    }
    if !commit_feito && git(&["log", "-1", "--format=%H"]).is_none_or(|h| h.is_empty()) {
        return Some(tr("Nothing committed yet: create the commit first.").into());
    }
    if git(&["remote", "get-url", "origin"]).is_none_or(|u| u.is_empty()) {
        return Some(tr("This repository has no remote (origin): there is nowhere to send it.").into());
    }
    let tem_gh = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("gh").is_file()));
    if !tem_gh {
        return Some(tr("The GitHub command (gh) is not installed: the pull request cannot be opened from here.").into());
    }
    None
}

/// git push -u origin BRANCH, then gh pr create --fill: the PR's address.
fn open_pr(root: &std::path::Path, ramo: &str) -> Result<String, String> {
    let run = |cmd: &str, args: &[&str]| -> Result<String, String> {
        let o = std::process::Command::new(cmd).args(args).current_dir(root).stdin(std::process::Stdio::null()).output().map_err(|e| e.to_string())?;
        let out = String::from_utf8_lossy(&o.stdout).trim().to_string();
        let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
        if o.status.success() { Ok(out) } else { Err(err.lines().last().unwrap_or("").to_string()) }
    };
    run("git", &["push", "-u", "origin", ramo])?;
    let out = run("gh", &["pr", "create", "--fill", "--head", ramo])?;
    Ok(out.lines().rev().find(|l| l.starts_with("http")).unwrap_or(&out).to_string())
}

/// A text's first whole sentences within `max` characters (plain: no bold marks or code ticks), "…" if cut inside one.
fn first_sentences(t: &str, max: usize) -> String {
    let plano = t.replace("**", "").replace('`', "").split_whitespace().collect::<Vec<_>>().join(" ");
    if plano.chars().count() <= max {
        return plano;
    }
    let corte: String = plano.chars().take(max).collect();
    match corte.rfind(['.', '!', '?']).filter(|&i| i > max / 3) {
        Some(i) => corte[..=i].to_string(),
        None => format!("{}…", corte.rsplit_once(' ').map(|(a, _)| a).unwrap_or(&corte)),
    }
}

fn git_root(p: &std::path::Path) -> Option<std::path::PathBuf> {
    let dir = if p.is_dir() { p.to_path_buf() } else { p.parent()?.to_path_buf() };
    let o = std::process::Command::new("git").arg("-C").arg(&dir).args(["rev-parse", "--show-toplevel"]).stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null()).output().ok()?;
    o.status.success().then(|| std::path::PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
}

fn agent_prefs_start_on_last() -> bool {
    crate::agent_prefs::load().start_on == "ultima"
}

#[cfg(test)]
mod tests {
    use super::first_sentences;

    #[test]
    fn whole_sentences() {
        assert_eq!(first_sentences("Curto.", 220), "Curto.");
        let t = "O arquivo **notas.txt** foi atualizado. A linha `linha tres` virou linha 3. O conteúdo completo agora é: linha um, linha 2 e linha 3, nada mais, nada menos, só isso mesmo.";
        assert_eq!(first_sentences(t, 90), "O arquivo notas.txt foi atualizado. A linha linha tres virou linha 3.");
        // no sentence end in reach: cut at a word, with "…"
        assert_eq!(first_sentences("uma frase muito longa sem ponto algum", 20), "uma frase muito…");
    }
}
