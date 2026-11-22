//! Agents center › Permissões (design "Permissões por agente"): one table for the agents, three levels (livre,
//! perguntar, nunca) per column (read, write, delete, network, terminal, see the screen, spend). A click on a cell
//! changes it at once, also for work already running (the agent reads its permissions before every action). On the
//! right, where the agent may act (its folders, and the ones kalangod protects from every agent), its limits (tokens a
//! day, hours, model), and pausing it or revoking everything.
//!
//! Below the table, for all agents (design page 30, "Permissões · arquivos que rodam ao entrar"): whether a change
//! to a file that runs at login (.bashrc, autostart…) asks every time or goes ahead; kalangod reads that choice and
//! enforces it.
//!
//! Everything comes from and goes to the agent runtime (`kalango-agente perfil`, `coluna`, `pasta`, `limite`,
//! `pausar`, `revogar`, `inicio`), which is what enforces it.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, estrela, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{n_, tr, trf};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::process::Command;
use std::rc::{Rc, Weak};
use std::time::Duration;

/// The table's columns: (id, name, glyph).
const COLUNAS: &[(&str, &str, &str)] = &[
    ("ler", n_("Read"), "olho"),
    ("escrever", n_("Write"), "doc"),
    ("apagar", n_("Delete"), "lixeira"),
    ("rede", n_("Network"), "globo"),
    ("terminal", n_("Terminal"), "terminal"),
    ("tela", n_("See the screen"), "monitor"),
    ("gastar", n_("Spend"), "raio"),
];

const HORARIOS: [&str; 3] = ["sempre", "dia", "noite"];

/// The choices for files that run at login: (value, title, what it means).
const INICIO: [(&str, &str, &str); 2] = [
    ("perguntar", n_("Ask every time (recommended)"), n_("each change shows what changes and asks for your approval")),
    ("livre", n_("Allow automatically"), n_("the change happens and goes to the timeline, with undo")),
];

/// A column unused for this long is suggested to become "never".
const DIAS_SEM_USO: u64 = 30;

fn proximo(v: &str) -> &'static str {
    match v {
        "livre" => "perguntar",
        "perguntar" => "nunca",
        _ => "livre",
    }
}

fn nome_nivel(v: &str) -> &'static str {
    match v {
        "livre" => tr("free"),
        "perguntar" => tr("ask"),
        _ => tr("never"),
    }
}

fn mil(n: u64) -> String {
    if n >= 1000 { format!("{}k", (n + 500) / 1000) } else { n.to_string() }
}

/// A path as the person reads it (under the home folder: with ~).
fn com_til(p: &std::path::Path) -> String {
    let home = glib::home_dir();
    match p.strip_prefix(&home) {
        Ok(r) if r.as_os_str().is_empty() => "~".into(),
        Ok(r) => format!("~/{}", r.display()),
        Err(_) => p.display().to_string(),
    }
}

/// The page's measures: the design's, or compact when the window is narrower than it (a notebook screen).
#[derive(Clone, Copy)]
struct Medidas {
    agente: i32,
    celula: i32,
    painel: i32,
    escala: i32,
    escolha: i32,
    inicio: i32,
}

impl Medidas {
    fn para(largura_janela: i32) -> Medidas {
        if largura_janela >= 1480 {
            Medidas { agente: 254, celula: 92, painel: 474, escala: 200, escolha: 170, inicio: 280 }
        } else {
            Medidas { agente: 196, celula: 74, painel: 340, escala: 130, escolha: 150, inicio: 240 }
        }
    }
}

pub struct Permissoes {
    pub raiz: gtk::Box,
    medidas: Medidas,
    shell: Weak<Shell>,
    linhas: gtk::Box,
    sugestao: gtk::Box,
    sugestao_texto: gtk::Label,
    sugestao_coluna: RefCell<String>,
    agente: gtk::Label,
    pastas: gtk::Box,
    tokens: gtk::Scale,
    tokens_v: gtk::Label,
    tokens_espera: RefCell<Option<glib::SourceId>>,
    horario: gtk::DropDown,
    pausar_glifo: gtk::Box,
    pausar_rotulo: gtk::Label,
    /// files that run at login: the chosen value's title and each choice's check mark
    inicio_valor: gtk::Label,
    inicio_marcas: Vec<gtk::Image>,
    inicio_opcoes: Vec<gtk::Button>,
    /// approval by the d1 (design page 31): the switch, the minimum confidence and what the switch says it does
    d1_chave: gtk::Switch,
    d1_sub: gtk::Label,
    d1_confianca: gtk::Box,
    d1_escala: gtk::Scale,
    d1_valor: gtk::Label,
    d1_espera: RefCell<Option<glib::SourceId>>,
    /// where agents' commands write (design page 32): the work folders and the project folders
    projetos: gtk::Box,
    perfil: RefCell<Value>,
    /// filling the controls: their signals are not the person's changes
    preenchendo: Cell<bool>,
}

impl Permissoes {
    pub fn new(shell: &Rc<Shell>, largura_janela: i32) -> Rc<Permissoes> {
        let m = Medidas::para(largura_janela);
        let raiz = caixa(Orientation::Horizontal, 0, "");
        raiz.set_vexpand(true);
        // ---- the table
        let corpo = caixa(Orientation::Vertical, 0, "central-corpo");
        corpo.set_hexpand(true);
        let h = rotulo(tr("Permissions"), "central-titulo");
        h.set_xalign(0.0);
        corpo.append(&h);
        let sub = rotulo(tr("What each agent can do without asking. Click a cell to change it."), "central-sub");
        sub.set_xalign(0.0);
        sub.set_margin_top(8);
        sub.set_margin_bottom(14);
        corpo.append(&sub);
        let tabela = caixa(Orientation::Vertical, 0, "central-tabela");
        let cab = caixa(Orientation::Horizontal, 0, "central-tabela-cab");
        let a = rotulo(tr("Agent"), "central-tabela-titulo");
        a.set_xalign(0.0);
        a.set_size_request(m.agente, -1);
        a.set_margin_start(13);
        cab.append(&a);
        for (_, nome, glifo) in COLUNAS {
            let c = caixa(Orientation::Vertical, 3, "");
            c.set_size_request(m.celula, -1);
            c.set_valign(Align::Center);
            let g = ui_icon(glifo, "#8E8F99", 14);
            g.set_halign(Align::Center);
            c.append(&g);
            c.append(&rotulo(tr(nome), "central-tabela-titulo"));
            cab.append(&c);
        }
        tabela.append(&cab);
        let linhas = caixa(Orientation::Vertical, 0, "");
        tabela.append(&linhas);
        corpo.append(&tabela);
        // the legend
        let legenda = caixa(Orientation::Horizontal, 18, "");
        legenda.set_margin_top(14);
        for (classe, nivel, explica) in [("livre", "livre", tr("does it without asking")), ("perguntar", "perguntar", tr("asks for approval every time")), ("nunca", "nunca", tr("blocked, does not even ask"))] {
            let l = caixa(Orientation::Horizontal, 6, "");
            let p = caixa(Orientation::Vertical, 0, "central-ponto");
            p.add_css_class(classe);
            p.set_valign(Align::Center);
            l.append(&p);
            l.append(&rotulo(nome_nivel(nivel), "central-legenda-nivel"));
            l.append(&rotulo(explica, "central-legenda"));
            legenda.append(&l);
        }
        corpo.append(&legenda);
        // a column long unused: "never"?
        let sugestao = caixa(Orientation::Horizontal, 9, "central-sugestao");
        sugestao.set_margin_top(14);
        sugestao.append(&estrela(14, "#A5A1E2"));
        let sugestao_texto = rotulo("", "central-sugestao-texto");
        sugestao_texto.set_hexpand(true);
        sugestao_texto.set_xalign(0.0);
        sugestao_texto.set_wrap(true);
        sugestao.append(&sugestao_texto);
        let mudar = gtk::Button::with_label(tr("Change"));
        mudar.add_css_class("central-sugestao-botao");
        mudar.set_valign(Align::Center);
        sugestao.append(&mudar);
        sugestao.set_visible(false);
        corpo.append(&sugestao);
        // ---- for all agents: files that run at login
        let todos = rotulo(tr("ALL AGENTS"), "central-painel-secao");
        todos.set_xalign(0.0);
        todos.set_margin_top(22);
        todos.set_margin_bottom(12);
        corpo.append(&todos);
        let cartao = caixa(Orientation::Vertical, 0, "central-cartao");
        let linha = caixa(Orientation::Horizontal, 12, "central-pasta");
        linha.append(&ui_icon("terminal", "#8E8F99", 16));
        let col = caixa(Orientation::Vertical, 0, "");
        col.set_hexpand(true);
        col.set_valign(Align::Center);
        let t = rotulo(tr("Files that run at login"), "central-pasta-nome");
        t.set_xalign(0.0);
        col.append(&t);
        let d = rotulo(tr(".bashrc, .profile, autostart, user services…"), "central-agente-sub");
        d.set_xalign(0.0);
        d.set_ellipsize(gtk::pango::EllipsizeMode::End);
        d.set_max_width_chars(1);
        col.append(&d);
        linha.append(&col);
        // the selector: a button with the chosen value, and a list of both with what each means
        let inicio_valor = rotulo("", "");
        inicio_valor.set_xalign(0.0);
        inicio_valor.set_hexpand(true);
        inicio_valor.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let bc = caixa(Orientation::Horizontal, 8, "");
        bc.append(&inicio_valor);
        bc.append(&ui_icon("chevron-baixo", "#8E8F99", 16));
        let seletor = gtk::MenuButton::new();
        seletor.add_css_class("central-inicio");
        seletor.set_child(Some(&bc));
        seletor.set_valign(Align::Center);
        seletor.set_size_request(m.inicio, -1);
        let lista = caixa(Orientation::Vertical, 2, "");
        let (mut inicio_marcas, mut inicio_opcoes) = (Vec::new(), Vec::new());
        for (_, titulo, explica) in INICIO {
            let b = gtk::Button::new();
            b.add_css_class("central-inicio-opcao");
            let r = caixa(Orientation::Horizontal, 10, "");
            let marca = ui_icon("check", "#A5A1E2", 16);
            marca.set_valign(Align::Start);
            r.append(&marca);
            let c = caixa(Orientation::Vertical, 2, "");
            let a = rotulo(tr(titulo), "central-pasta-nome");
            a.set_xalign(0.0);
            c.append(&a);
            let e = rotulo(tr(explica), "central-agente-sub");
            e.set_xalign(0.0);
            e.set_wrap(true);
            e.set_max_width_chars(30);
            c.append(&e);
            r.append(&c);
            b.set_child(Some(&r));
            lista.append(&b);
            inicio_marcas.push(marca);
            inicio_opcoes.push(b);
        }
        let pop = gtk::Popover::new();
        pop.add_css_class("central-inicio-lista");
        pop.set_has_arrow(false);
        pop.set_position(gtk::PositionType::Bottom);
        pop.set_child(Some(&lista));
        seletor.set_popover(Some(&pop));
        linha.append(&seletor);
        cartao.append(&linha);
        corpo.append(&cartao);
        let ajuda = rotulo(tr("These files run when you log in or open a terminal: each change an agent makes shows what changes and asks for your approval, without “Always”."), "central-legenda");
        ajuda.set_xalign(0.0);
        ajuda.set_wrap(true);
        ajuda.set_max_width_chars(1);
        ajuda.set_hexpand(true);
        ajuda.set_margin_top(12);
        ajuda.add_css_class("central-inicio-ajuda");
        corpo.append(&ajuda);
        // ---- approval by the d1 (design page 31, "Aprovação com o d1"): off until the person turns it on
        let titulo_d1 = rotulo(tr("APPROVAL BY THE d1"), "central-painel-secao");
        titulo_d1.set_xalign(0.0);
        titulo_d1.set_margin_top(22);
        titulo_d1.set_margin_bottom(12);
        corpo.append(&titulo_d1);
        let cartao_d1 = caixa(Orientation::Vertical, 0, "central-cartao");
        let linha_d1 = caixa(Orientation::Horizontal, 12, "central-pasta");
        linha_d1.append(&ui_icon("raio", "#8E8F99", 16));
        let col_d1 = caixa(Orientation::Vertical, 0, "");
        col_d1.set_hexpand(true);
        col_d1.set_valign(Align::Center);
        let t_d1 = rotulo(tr("Approve by itself what is low risk"), "central-pasta-nome");
        t_d1.set_xalign(0.0);
        t_d1.set_ellipsize(gtk::pango::EllipsizeMode::End);
        t_d1.set_max_width_chars(1);
        col_d1.append(&t_d1);
        let d1_sub = rotulo("", "central-agente-sub");
        d1_sub.set_xalign(0.0);
        d1_sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
        d1_sub.set_max_width_chars(1);
        col_d1.append(&d1_sub);
        linha_d1.append(&col_d1);
        let d1_chave = gtk::Switch::new();
        d1_chave.add_css_class("central-switch");
        d1_chave.set_valign(Align::Center);
        linha_d1.append(&d1_chave);
        cartao_d1.append(&linha_d1);
        let d1_confianca = caixa(Orientation::Horizontal, 12, "central-limite");
        d1_confianca.append(&ui_icon("relogio", "#8E8F99", 16));
        let n_d1 = rotulo(tr("Minimum confidence"), "central-limite-nome");
        n_d1.set_xalign(0.0);
        n_d1.set_hexpand(true);
        n_d1.set_wrap(true);
        n_d1.set_max_width_chars(1);
        d1_confianca.append(&n_d1);
        let d1_escala = gtk::Scale::with_range(Orientation::Horizontal, 50.0, 99.0, 1.0);
        d1_escala.set_draw_value(false);
        d1_escala.set_size_request(m.escala, -1);
        d1_escala.add_css_class("central-escala");
        d1_confianca.append(&d1_escala);
        let d1_valor = rotulo("", "central-limite-valor");
        d1_valor.set_size_request(40, -1);
        d1_valor.set_xalign(1.0);
        d1_confianca.append(&d1_valor);
        cartao_d1.append(&d1_confianca);
        corpo.append(&cartao_d1);
        let ajuda_d1 = rotulo(tr("It may approve by itself: moving to the trash, creating a new file in your folders, renaming and moving inside your home folder. Everything goes to the timeline, with undo. If the d1 takes more than 1.5 s, or if you deny 3 times in a row, it goes back to asking."), "central-legenda");
        ajuda_d1.set_xalign(0.0);
        ajuda_d1.set_wrap(true);
        ajuda_d1.set_max_width_chars(1);
        ajuda_d1.set_hexpand(true);
        ajuda_d1.set_margin_top(12);
        ajuda_d1.add_css_class("central-inicio-ajuda");
        corpo.append(&ajuda_d1);
        // what it never approves: the chips wrap with the width
        let nunca = caixa(Orientation::Horizontal, 0, "central-nunca-lista");
        crate::wrap_row::WrapRow::apply(&nunca, 6, 6);
        nunca.set_margin_top(12);
        let tn = rotulo(tr("Never approves by itself:"), "central-nunca-titulo");
        tn.set_margin_end(4);
        nunca.append(&tn);
        for t in [tr("high-risk actions"), tr("system and full access"), tr("what cannot be undone"), tr("files that run at login"), tr("screen control"),
            tr("creating agents, flows and connectors"), tr("network and connectors"), tr("packages"), tr("a request coming from a page or file that was read")] {
            let c = rotulo(t, "central-nunca-chip");
            c.set_halign(Align::Start);
            nunca.append(&c);
        }
        corpo.append(&nunca);
        // ---- where agents' commands write (design page 32)
        let titulo_pastas = rotulo(tr("COMMAND FOLDERS"), "central-painel-secao");
        titulo_pastas.set_xalign(0.0);
        titulo_pastas.set_margin_top(22);
        titulo_pastas.set_margin_bottom(12);
        corpo.append(&titulo_pastas);
        let projetos = caixa(Orientation::Vertical, 0, "central-cartao");
        corpo.append(&projetos);
        let adicionar_projeto = gtk::Button::new();
        adicionar_projeto.add_css_class("central-adicionar");
        let apc = caixa(Orientation::Horizontal, 8, "");
        apc.append(&ui_icon("mais", "#C9CAD1", 14));
        apc.append(&rotulo(tr("Add project folder"), ""));
        adicionar_projeto.set_child(Some(&apc));
        adicionar_projeto.set_halign(Align::Start);
        adicionar_projeto.set_margin_top(14);
        corpo.append(&adicionar_projeto);
        let ajuda_pastas = rotulo(tr("Agents' commands write only in these folders and in a temporary folder of their own task; in the rest of your home they only read, and the protected folders not even that. Installing something for your user (pip --user, npm -g, cargo install) asks first, one command at a time."), "central-legenda");
        ajuda_pastas.set_xalign(0.0);
        ajuda_pastas.set_wrap(true);
        ajuda_pastas.set_max_width_chars(1);
        ajuda_pastas.set_hexpand(true);
        ajuda_pastas.set_margin_top(12);
        ajuda_pastas.add_css_class("central-inicio-ajuda");
        corpo.append(&ajuda_pastas);
        // (the table, the legend and the setting for all agents scroll: on a short screen the page must not make the
        // window taller than the output — a window keeps its biggest size, and the pages after it would be cut)
        let rolagem = gtk::ScrolledWindow::new();
        rolagem.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        rolagem.set_hexpand(true);
        rolagem.set_vexpand(true);
        rolagem.set_child(Some(&corpo));
        raiz.append(&rolagem);

        // ---- the chosen agent, on the right
        let painel = caixa(Orientation::Vertical, 0, "central-painel");
        painel.set_size_request(m.painel, -1);
        painel.set_hexpand(false);
        let quem = caixa(Orientation::Horizontal, 7, "");
        quem.append(&estrela(14, "#A5A1E2"));
        let agente = rotulo("", "central-painel-agente");
        quem.append(&agente);
        painel.append(&quem);
        let secao = |t: &str| {
            let l = rotulo(t, "central-painel-secao");
            l.set_xalign(0.0);
            l
        };
        let onde = secao(tr("WHERE IT CAN ACT"));
        onde.set_margin_top(14);
        onde.set_margin_bottom(12);
        painel.append(&onde);
        let pastas = caixa(Orientation::Vertical, 0, "central-cartao");
        painel.append(&pastas);
        let adicionar = gtk::Button::new();
        adicionar.add_css_class("central-adicionar");
        let ac = caixa(Orientation::Horizontal, 8, "");
        ac.append(&ui_icon("mais", "#C9CAD1", 14));
        ac.append(&rotulo(tr("Add folder"), ""));
        adicionar.set_child(Some(&ac));
        adicionar.set_halign(Align::Start);
        adicionar.set_margin_top(14);
        painel.append(&adicionar);
        let lim = secao(tr("LIMITS"));
        lim.set_margin_top(18);
        lim.set_margin_bottom(12);
        painel.append(&lim);
        let limites = caixa(Orientation::Vertical, 0, "central-cartao");
        let linha_limite = |glifo: &str, nome: &str, controle: &gtk::Widget| {
            let l = caixa(Orientation::Horizontal, 12, "central-limite");
            l.append(&ui_icon(glifo, "#8E8F99", 16));
            let n = rotulo(nome, "central-limite-nome");
            n.set_xalign(0.0);
            n.set_hexpand(true);
            // (it wraps: its natural width must not widen the panel)
            n.set_wrap(true);
            n.set_max_width_chars(1);
            l.append(&n);
            l.append(controle);
            limites.append(&l);
        };
        let tc = caixa(Orientation::Horizontal, 10, "");
        let tokens = gtk::Scale::with_range(Orientation::Horizontal, 0.0, 500_000.0, 10_000.0);
        tokens.set_draw_value(false);
        tokens.set_size_request(m.escala, -1);
        tokens.add_css_class("central-escala");
        tc.append(&tokens);
        let tokens_v = rotulo("", "central-limite-valor");
        tokens_v.set_size_request(56, -1);
        tokens_v.set_xalign(1.0);
        tc.append(&tokens_v);
        linha_limite("raio", tr("Tokens a day"), tc.upcast_ref());
        let horario = gtk::DropDown::from_strings(&[tr("at any time"), tr("from 8 am to 6 pm"), tr("from 6 pm to 8 am")]);
        horario.add_css_class("central-escolha");
        horario.set_valign(Align::Center);
        horario.set_size_request(m.escolha, -1);
        linha_limite("relogio", tr("May run"), horario.upcast_ref());
        // the cloud: no provider is set up in this build, so only the local model
        let modelo = gtk::DropDown::from_strings(&[tr("local only")]);
        modelo.add_css_class("central-escolha");
        modelo.set_valign(Align::Center);
        modelo.set_size_request(m.escolha, -1);
        modelo.set_sensitive(false);
        modelo.set_tooltip_text(Some(tr("No cloud provider is set up, so the agent only uses the local model.")));
        linha_limite("cpu", tr("Model"), modelo.upcast_ref());
        painel.append(&limites);
        let espaco = caixa(Orientation::Vertical, 0, "");
        espaco.set_vexpand(true);
        painel.append(&espaco);
        let botoes = caixa(Orientation::Horizontal, 8, "");
        botoes.set_homogeneous(true);
        let pausar = gtk::Button::new();
        pausar.add_css_class("central-botao");
        let pc = caixa(Orientation::Horizontal, 8, "");
        pc.set_halign(Align::Center);
        let pausar_glifo = caixa(Orientation::Horizontal, 0, "");
        pc.append(&pausar_glifo);
        let pausar_rotulo = rotulo("", "");
        pc.append(&pausar_rotulo);
        pausar.set_child(Some(&pc));
        botoes.append(&pausar);
        let revogar = gtk::Button::with_label(tr("Revoke everything"));
        revogar.add_css_class("central-botao");
        revogar.add_css_class("perigo");
        botoes.append(&revogar);
        painel.append(&botoes);
        raiz.append(&painel);

        let p = Rc::new(Permissoes {
            raiz, medidas: m, shell: Rc::downgrade(shell), linhas, sugestao, sugestao_texto, sugestao_coluna: RefCell::default(), agente, pastas,
            tokens, tokens_v, tokens_espera: RefCell::default(), horario, pausar_glifo, pausar_rotulo, inicio_valor, inicio_marcas, inicio_opcoes, d1_chave, d1_sub, d1_confianca, d1_escala, d1_valor, d1_espera: RefCell::default(), projetos, perfil: RefCell::new(Value::Null),
            preenchendo: Cell::new(false),
        });
        for (i, b) in p.inicio_opcoes.iter().enumerate() {
            let (w, pop) = (Rc::downgrade(&p), pop.clone());
            b.connect_clicked(move |_| {
                pop.popdown();
                if let Some(p) = w.upgrade() {
                    p.executar(vec!["inicio".into(), INICIO[i].0.into()]);
                }
            });
        }
        crate::central::posicao_debug("arquivos de início", seletor.upcast_ref());
        let w = Rc::downgrade(&p);
        p.d1_chave.connect_state_set(move |_, on| {
            if let Some(p) = w.upgrade() {
                p.d1_dim(on);
                if !p.preenchendo.get() {
                    p.executar(vec!["aprovacao-d1".into(), if on { "ligar" } else { "desligar" }.into()]);
                }
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(&p);
        p.d1_escala.connect_value_changed(move |sc| {
            let Some(p) = w.upgrade() else { return };
            let n = sc.value().round() as u64;
            p.d1_valor.set_text(&format!("{n} %"));
            if p.preenchendo.get() {
                return;
            }
            // saved when the slider rests
            if let Some(id) = p.d1_espera.borrow_mut().take() {
                id.remove();
            }
            let w = Rc::downgrade(&p);
            *p.d1_espera.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(450), move || {
                if let Some(p) = w.upgrade() {
                    p.d1_espera.borrow_mut().take();
                    p.executar(vec!["aprovacao-d1".into(), "confianca".into(), n.to_string()]);
                }
            }));
        });
        let w = Rc::downgrade(&p);
        adicionar_projeto.connect_clicked(move |_| {
            let Some(p) = w.upgrade() else { return };
            let w = Rc::downgrade(&p);
            crate::arquivos::seletor::escolher_pasta(tr("Agents"), tr("Add project folder"), Some(glib::home_dir()), move |caminho| {
                let Some(p) = w.upgrade() else { return };
                let w = Rc::downgrade(&p);
                let c = caminho.display().to_string();
                // (kalangod asks on the approval screen: an agent cannot add one by calling the same method)
                crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_pastas_adicionar(&c), move |r| {
                    if let Err(e) = r {
                        eprintln!("kalango-shell: adicionar pasta de projeto: {e}");
                    }
                    if let Some(p) = w.upgrade() {
                        p.recarregar_pastas();
                    }
                });
            });
        });
        let w = Rc::downgrade(&p);
        mudar.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                let c = p.sugestao_coluna.borrow().clone();
                p.executar(vec!["coluna".into(), c, "nunca".into()]);
            }
        });
        let w = Rc::downgrade(&p);
        adicionar.connect_clicked(move |_| {
            let Some(p) = w.upgrade() else { return };
            let w = Rc::downgrade(&p);
            crate::arquivos::seletor::escolher_pasta(tr("Agents"), tr("Add folder"), Some(glib::home_dir()), move |caminho| {
                if let Some(p) = w.upgrade() {
                    p.executar(vec!["pasta".into(), com_til(&caminho), "adicionar".into()]);
                }
            });
        });
        let w = Rc::downgrade(&p);
        p.tokens.connect_value_changed(move |s| {
            let Some(p) = w.upgrade() else { return };
            let n = s.value().round() as u64;
            p.tokens_v.set_text(&if n == 0 { tr("no limit").to_string() } else { mil(n) });
            if p.preenchendo.get() {
                return;
            }
            // saved when the slider rests
            if let Some(id) = p.tokens_espera.borrow_mut().take() {
                id.remove();
            }
            let w = Rc::downgrade(&p);
            *p.tokens_espera.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(450), move || {
                if let Some(p) = w.upgrade() {
                    p.tokens_espera.borrow_mut().take();
                    p.executar(vec!["limite".into(), "tokens".into(), n.to_string()]);
                }
            }));
        });
        let w = Rc::downgrade(&p);
        p.horario.connect_selected_notify(move |d| {
            let Some(p) = w.upgrade() else { return };
            if !p.preenchendo.get() {
                let h = HORARIOS.get(d.selected() as usize).copied().unwrap_or("sempre");
                p.executar(vec!["limite".into(), "horario".into(), h.into()]);
            }
        });
        let w = Rc::downgrade(&p);
        pausar.connect_clicked(move |_| {
            let Some(p) = w.upgrade() else { return };
            let pausado = p.perfil.borrow()["pausado"].as_bool().unwrap_or(false);
            // work already running stops (or goes on) now, not only the next request
            if let Some(s) = p.shell.upgrade() {
                let conv = s.right_bar().conversa.clone();
                if conv.work().is_some() {
                    conv.pause(!pausado);
                }
            }
            p.executar(vec!["pausar".into(), if pausado { "nao" } else { "sim" }.into()]);
        });
        let w = Rc::downgrade(&p);
        revogar.connect_clicked(move |_| {
            let Some(p) = w.upgrade() else { return };
            // also the "always" answers the person gave the agent on the system approval screen
            crate::lateral::em_segundo_plano(|| crate::servicos::kalangod_revogar_sempre_de("kalango-agente"), move |r| {
                if let Err(e) = r {
                    eprintln!("kalango-shell: revogar as respostas \"sempre\" do agente: {e}");
                }
                p.executar(vec!["revogar".into()]);
            });
        });
        p
    }

    /// Reads where agents' commands write (kalangod) and shows it.
    fn recarregar_pastas(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(crate::servicos::kalangod_pastas, move |r| {
            let Some(p) = w.upgrade() else { return };
            match r {
                Ok(v) => p.render_pastas(&v),
                Err(e) => eprintln!("kalango-shell: pastas dos comandos: {e}"),
            }
        });
    }

    /// The command folders card: the work folders ("always"), then each project folder with how it got there.
    fn render_pastas(self: &Rc<Self>, v: &Value) {
        while let Some(x) = self.projetos.first_child() {
            self.projetos.remove(&x);
        }
        let linha = |titulo: &str, sub: &str| {
            let l = caixa(Orientation::Horizontal, 12, "central-pasta");
            l.append(&ui_icon("pasta", "#8E8F99", 16));
            let col = caixa(Orientation::Vertical, 0, "");
            col.set_hexpand(true);
            col.set_valign(Align::Center);
            let n = rotulo(titulo, "central-pasta-nome");
            n.set_xalign(0.0);
            n.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            n.set_max_width_chars(1);
            col.append(&n);
            let d = rotulo(sub, "central-agente-sub");
            d.set_xalign(0.0);
            d.set_ellipsize(gtk::pango::EllipsizeMode::End);
            d.set_max_width_chars(1);
            col.append(&d);
            l.append(&col);
            l
        };
        let nomes = |p: &str| p.trim_start_matches("~/").to_string();
        let trabalho: Vec<String> = v["trabalho"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(nomes)).collect();
        let l = linha(tr("Work folders"), &if trabalho.is_empty() { tr("none found").to_string() } else { trabalho.join(" · ") });
        let chip = rotulo(tr("always"), "central-celula");
        chip.add_css_class("livre");
        chip.set_valign(Align::Center);
        l.append(&chip);
        self.projetos.append(&l);
        let projetos = v["projetos"].as_array().cloned().unwrap_or_default();
        for pr in &projetos {
            let caminho = pr["caminho"].as_str().unwrap_or("").to_string();
            let quando = glib::DateTime::from_unix_local(pr["criado"].as_i64().unwrap_or(0)).ok()
                .and_then(|d| d.format("%-d %b").ok()).map(|s| s.to_lowercase().trim_end_matches('.').to_string()).unwrap_or_default();
            let sub = if pr["origem"] == "adicionada" {
                trf("project folder · added by you on {date}", &[("date", quando)])
            } else {
                trf("project folder · approved by you on {date}", &[("date", quando)])
            };
            let l = linha(&caminho, &sub);
            let remover = gtk::Button::with_label(tr("Remove"));
            remover.add_css_class("central-remover");
            remover.set_valign(Align::Center);
            let w = Rc::downgrade(self);
            remover.connect_clicked(move |_| {
                let c = caminho.clone();
                let w = w.clone();
                crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_pastas_remover(&c), move |r| {
                    if let Err(e) = r {
                        eprintln!("kalango-shell: remover pasta de projeto: {e}");
                    }
                    if let Some(p) = w.upgrade() {
                        p.recarregar_pastas();
                    }
                });
            });
            l.append(&remover);
            self.projetos.append(&l);
        }
        // the folders approved for one task, until they end (at most 30 min, or when the asking program ends)
        let tarefas = v["tarefa"].as_array().cloned().unwrap_or_default();
        for t in &tarefas {
            let caminho = t["caminho"].as_str().unwrap_or("").to_string();
            let ate = glib::DateTime::from_unix_local(t["ate"].as_i64().unwrap_or(0)).ok().and_then(|d| d.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default();
            let l = linha(&caminho, &trf("only this task · until {time}", &[("time", ate)]));
            let remover = gtk::Button::with_label(tr("Remove"));
            remover.add_css_class("central-remover");
            remover.set_valign(Align::Center);
            let w = Rc::downgrade(self);
            remover.connect_clicked(move |_| {
                let c = caminho.clone();
                let w = w.clone();
                crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_pastas_remover(&c), move |r| {
                    if let Err(e) = r {
                        eprintln!("kalango-shell: remover pasta da tarefa: {e}");
                    }
                    if let Some(p) = w.upgrade() {
                        p.recarregar_pastas();
                    }
                });
            });
            l.append(&remover);
            self.projetos.append(&l);
        }
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug central: pastas dos comandos ({}; {} de projeto; {} da tarefa)", trabalho.join(", "), projetos.len(), tarefas.len());
        }
    }

    /// Reads the agent's permissions again (in the background) and shows them.
    pub fn recarregar(self: &Rc<Self>) {
        self.recarregar_pastas();
        let w = Rc::downgrade(self);
        let bin = crate::voz::agent_binary();
        crate::lateral::em_segundo_plano(move || {
            Command::new(&bin).arg("perfil").output().ok().filter(|o| o.status.success()).and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
        }, move |v| {
            let Some(p) = w.upgrade() else { return };
            match v {
                Some(v) => {
                    *p.perfil.borrow_mut() = v;
                    p.render();
                }
                None => eprintln!("kalango-shell: kalango-agente perfil não respondeu"),
            }
        });
    }

    /// The minimum-confidence row is dimmed while the d1 is not allowed to approve (design: the off state).
    fn d1_dim(&self, on: bool) {
        self.d1_confianca.set_opacity(if on { 1.0 } else { 0.45 });
        self.d1_escala.set_sensitive(on);
        self.d1_sub.set_text(&if on { tr("the d1 judges each low-risk action").to_string() } else { tr("the d1 judges each low-risk action · off until you turn it on").to_string() });
    }

    /// A change (a `kalango-agente` subcommand), then the page again.
    fn executar(self: &Rc<Self>, args: Vec<String>) {
        let w = Rc::downgrade(self);
        let bin = crate::voz::agent_binary();
        crate::lateral::em_segundo_plano(move || Command::new(&bin).args(&args).output().map_err(|e| e.to_string()).and_then(|o| {
            if o.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_string()) }
        }), move |r| {
            if let Err(e) = r {
                eprintln!("kalango-shell: permissões do agente: {e}");
            }
            if let Some(p) = w.upgrade() {
                p.recarregar();
            }
        });
    }

    fn render(self: &Rc<Self>) {
        let v = self.perfil.borrow().clone();
        self.preenchendo.set(true);
        let pausado = v["pausado"].as_bool().unwrap_or(false);
        // the table: the system's agent (the one in the side bar and in the routines)
        while let Some(x) = self.linhas.first_child() {
            self.linhas.remove(&x);
        }
        let linha = caixa(Orientation::Horizontal, 0, "central-tabela-linha");
        linha.add_css_class("escolhida");
        let quem = caixa(Orientation::Horizontal, 10, "");
        quem.set_size_request(self.medidas.agente, -1);
        quem.set_margin_start(12);
        let av = caixa(Orientation::Horizontal, 0, "central-avatar");
        av.set_size_request(30, 30);
        av.set_valign(Align::Center);
        // (the centred star must not stretch it)
        av.set_hexpand(false);
        let sp = estrela(14, "#A5A1E2");
        sp.set_hexpand(true);
        sp.set_halign(Align::Center);
        av.append(&sp);
        quem.append(&av);
        let col = caixa(Orientation::Vertical, 1, "");
        col.set_valign(Align::Center);
        let n = rotulo(tr("Agent"), "central-agente-nome");
        n.set_xalign(0.0);
        col.append(&n);
        let inicio_do_dia = glib::DateTime::now_local().ok()
            .and_then(|d| glib::DateTime::from_local(d.year(), d.month(), d.day_of_month(), 0, 0, 0.0).ok()).map(|d| d.to_unix()).unwrap_or(0);
        let hoje = crate::conversa::tarefas(200).iter().filter(|t| t["fim"].as_i64().unwrap_or(0) >= inicio_do_dia).count();
        let tarefas = if hoje == 1 { tr("1 task today").to_string() } else { trf("{n} tasks today", &[("n", hoje.to_string())]) };
        let s = rotulo(&format!("{} · {}", if pausado { tr("paused") } else { tr("local") }, tarefas), "central-agente-sub");
        s.set_xalign(0.0);
        col.append(&s);
        quem.append(&col);
        linha.append(&quem);
        let colunas = v["colunas"].as_array().cloned().unwrap_or_default();
        for (id, nome, _) in COLUNAS {
            let c = colunas.iter().find(|c| c["id"] == *id).cloned().unwrap_or_default();
            let valor = c["valor"].as_str().unwrap_or("perguntar").to_string();
            let cel = caixa(Orientation::Horizontal, 0, "");
            cel.set_size_request(self.medidas.celula, -1);
            cel.set_hexpand(false);
            let b = gtk::Button::with_label(nome_nivel(&valor));
            b.add_css_class("central-celula");
            b.add_css_class(&valor);
            b.set_halign(Align::Center);
            b.set_valign(Align::Center);
            b.set_hexpand(true);
            let mut dica = trf("{column}: {level}. Click to change to “{next}”.", &[("column", tr(nome).to_string()), ("level", nome_nivel(&valor).to_string()), ("next", nome_nivel(proximo(&valor)).to_string())]);
            if valor == "livre" && c["sistema"].as_bool().unwrap_or(false) {
                dica.push('\n');
                dica.push_str(tr("Changes to the system still ask for approval on the system screen."));
            }
            b.set_tooltip_text(Some(&dica));
            let w = Rc::downgrade(self);
            let id = id.to_string();
            crate::central::posicao_debug(&format!("célula {id}"), b.upcast_ref());
            b.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    p.executar(vec!["coluna".into(), id.clone(), proximo(&valor).into()]);
                }
            });
            cel.append(&b);
            linha.append(&cel);
        }
        self.linhas.append(&linha);
        self.agente.set_text(&tr("Agent").to_uppercase());

        // a column with tools, not "never", unused for a long time
        let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0) as u64;
        let desde = v["uso"]["desde"].as_u64().unwrap_or(agora);
        let parada = COLUNAS.iter().filter_map(|(id, nome, _)| {
            let c = colunas.iter().find(|c| c["id"] == *id)?;
            if c["valor"] == "nunca" || c["ferramentas"].as_u64().unwrap_or(0) == 0 {
                return None;
            }
            let ultimo = v["uso"]["ultimo"][*id].as_u64().unwrap_or(desde);
            let dias = agora.saturating_sub(ultimo) / 86_400;
            (dias >= DIAS_SEM_USO).then_some((*id, *nome, dias))
        }).max_by_key(|(_, _, d)| *d);
        match parada {
            Some((id, nome, dias)) => {
                self.sugestao_texto.set_text(&trf("The agent has not used “{column}” for {n} days. Change it to “never”?", &[("column", tr(nome).to_string()), ("n", dias.to_string())]));
                *self.sugestao_coluna.borrow_mut() = id.to_string();
                self.sugestao.set_visible(true);
            }
            None => self.sugestao.set_visible(false),
        }

        // the folders, then the ones no agent reaches
        while let Some(x) = self.pastas.first_child() {
            self.pastas.remove(&x);
        }
        for f in v["pastas"].as_array().cloned().unwrap_or_default() {
            let caminho = f["caminho"].as_str().unwrap_or("").to_string();
            let escreve = f["modo"] != "ler";
            let l = caixa(Orientation::Horizontal, 12, "central-pasta");
            l.append(&ui_icon("pasta", "#8E8F99", 16));
            let col = caixa(Orientation::Vertical, 0, "");
            col.set_hexpand(true);
            col.set_valign(Align::Center);
            let nome = rotulo(&if caminho == "~" { tr("~ (your home folder)").to_string() } else { caminho.clone() }, "central-pasta-nome");
            nome.set_xalign(0.0);
            nome.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            nome.set_max_width_chars(1);
            col.append(&nome);
            let modo = gtk::Button::with_label(if escreve { tr("read and change") } else { tr("read only") });
            modo.add_css_class("central-pasta-modo");
            modo.set_halign(Align::Start);
            modo.set_tooltip_text(Some(tr("Click to change")));
            let (w, c2) = (Rc::downgrade(self), caminho.clone());
            modo.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    p.executar(vec!["pasta".into(), c2.clone(), if escreve { "ler" } else { "mexer" }.into()]);
                }
            });
            col.append(&modo);
            l.append(&col);
            let sw = gtk::Switch::new();
            sw.add_css_class("central-switch");
            sw.set_active(f["ativa"].as_bool().unwrap_or(true));
            sw.set_valign(Align::Center);
            let w = Rc::downgrade(self);
            sw.connect_state_set(move |_, on| {
                if let Some(p) = w.upgrade() {
                    if !p.preenchendo.get() {
                        p.executar(vec!["pasta".into(), caminho.clone(), if on { "ativa" } else { "desativada" }.into()]);
                    }
                }
                glib::Propagation::Proceed
            });
            l.append(&sw);
            self.pastas.append(&l);
        }
        for caminho in v["protegidas"].as_array().cloned().unwrap_or_default() {
            let Some(caminho) = caminho.as_str() else { continue };
            let l = caixa(Orientation::Horizontal, 12, "central-pasta");
            l.append(&ui_icon("cadeado", "#8E8F99", 16));
            let col = caixa(Orientation::Vertical, 0, "");
            col.set_hexpand(true);
            col.set_valign(Align::Center);
            let nome = rotulo(caminho, "central-pasta-nome");
            nome.set_xalign(0.0);
            nome.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            nome.set_max_width_chars(1);
            col.append(&nome);
            let d = rotulo(tr("protected: no agent reads it"), "central-agente-sub");
            d.set_xalign(0.0);
            d.set_wrap(true);
            d.set_max_width_chars(1);
            col.append(&d);
            l.append(&col);
            let chip = rotulo(nome_nivel("nunca"), "central-celula");
            chip.add_css_class("nunca");
            chip.set_valign(Align::Center);
            l.append(&chip);
            self.pastas.append(&l);
        }

        // limits
        self.tokens.set_value(v["tokens_dia"].as_u64().unwrap_or(0) as f64);
        self.tokens_v.set_text(&match v["tokens_dia"].as_u64() { Some(n) => mil(n), None => tr("no limit").to_string() });
        self.tokens.set_tooltip_text(Some(&trf("{n} tokens used today", &[("n", mil(v["tokens_hoje"].as_u64().unwrap_or(0)))])));
        let h = v["horario"].as_str().unwrap_or("sempre");
        self.horario.set_selected(HORARIOS.iter().position(|x| *x == h).unwrap_or(0) as u32);
        while let Some(x) = self.pausar_glifo.first_child() {
            self.pausar_glifo.remove(&x);
        }
        self.pausar_glifo.append(&ui_icon(if pausado { "tocar" } else { "pausa" }, "#E2E2E6", 14));
        self.pausar_rotulo.set_text(if pausado { tr("Resume agent") } else { tr("Pause agent") });
        let d1_ligado = v["aprovacao_d1"]["ligada"].as_bool().unwrap_or(false);
        let d1_confianca = v["aprovacao_d1"]["confianca"].as_u64().unwrap_or(90).clamp(50, 99);
        self.d1_chave.set_active(d1_ligado);
        self.d1_escala.set_value(d1_confianca as f64);
        self.d1_valor.set_text(&format!("{d1_confianca} %"));
        self.d1_dim(d1_ligado);
        let inicio = v["arquivos_de_inicio"].as_str().unwrap_or("perguntar");
        let i = INICIO.iter().position(|(x, _, _)| *x == inicio).unwrap_or(0);
        self.inicio_valor.set_text(tr(INICIO[i].1));
        for (j, (m, b)) in self.inicio_marcas.iter().zip(&self.inicio_opcoes).enumerate() {
            m.set_opacity(if j == i { 1.0 } else { 0.0 });
            if j == i { b.add_css_class("escolhida") } else { b.remove_css_class("escolhida") }
        }
        if let Some(b) = self.pausar_glifo.parent().and_then(|c| c.parent()) {
            crate::central::posicao_debug("pausar", &b);
        }
        self.preenchendo.set(false);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            let valores: Vec<String> = colunas.iter().map(|c| format!("{}={}", c["id"].as_str().unwrap_or(""), c["valor"].as_str().unwrap_or(""))).collect();
            eprintln!("debug central: permissões ({}; {} pastas; pausado {pausado}; arquivos de início {inicio})", valores.join(" "), v["pastas"].as_array().map(|a| a.len()).unwrap_or(0));
        }
    }
}
