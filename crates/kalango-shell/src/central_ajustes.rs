//! Agents center › Ajustes › Conectores (MCP) (design "Ajustes · Conectores (MCP)"): how the agents reach apps and
//! services. Two kinds of connector:
//! - built in: kalangod's own tools, grouped (files, terminal, packages…), always local; turning one off sets its
//!   tools to "never" for the agent (on again: their defaults);
//! - third party: kalangod manifests in `~/.config/kalango/conectores/<name>.toml` (a command run isolated, or a remote
//!   HTTP endpoint with OAuth); on/off in `conectores.json`; their state comes from a real handshake
//!   (`kalangod conector testar`), "Entrar" signs in (`kalangod conector entrar`), the log is kalangod's record of
//!   every call.
//!
//! Each tool's permission (free, ask, never) is the agent's own (a click changes it). "Importar configuração" reads the
//! usual MCP JSON (`{"mcpServers": {…}}`) and writes one manifest per server; "Adicionar conector" writes one from a
//! name and a command or address, after testing it.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, estrela, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{gio, glib, Align, Orientation};
use kalango_i18n::{n_, tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

/// kalangod's tools for its own undo: the agent never offers them, the page does not list them.
const INTERNAS: [&str; 2] = ["arquivos.restaurar", "arquivos.apagar_criado"];

/// kalangod's own tools as built-in connectors: (id, name, glyph, tool prefixes).
const EMBUTIDOS: &[(&str, &str, &str, &[&str])] = &[
    ("arquivos", n_("System files"), "pasta", &["arquivos."]),
    ("terminal", n_("Terminal and processes"), "terminal", &["tarefas.", "processos.", "sistema.executar"]),
    ("pacotes", n_("Packages and updates"), "baixar", &["pacotes.", "atualizacoes."]),
    ("rede", n_("Network and firewall"), "wifi", &["rede.", "firewall."]),
    ("discos", n_("Disks"), "cpu", &["discos."]),
    ("usuarios", n_("Accounts"), "usuario", &["usuarios."]),
    ("sistema", n_("Services and system"), "engrenagem", &["servicos.", "sistema.info", "kernel.", "sysctl.", "registros."]),
    ("pontos", n_("Restore points and timeline"), "relogio", &["pontos.", "linha_do_tempo."]),
    ("web", n_("Web"), "globo", &["web."]),
];

pub(crate) fn pasta() -> PathBuf {
    glib::user_config_dir().join("kalango/conectores")
}

pub(crate) fn kalangod_bin() -> PathBuf {
    if let Some(p) = std::env::var_os("KALANGOD_BIN") {
        return p.into();
    }
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("kalangod"))).filter(|p| p.is_file()).unwrap_or_else(|| "/usr/bin/kalangod".into())
}

fn estado() -> Value {
    std::fs::read_to_string(pasta().with_extension("json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(json!({}))
}

fn set_ligado(nome: &str, on: bool) {
    let mut v = estado();
    v[nome] = json!({"ligado": on});
    let p = pasta().with_extension("json");
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, serde_json::to_string_pretty(&v).unwrap_or_default());
}

/// A valid connector name from any text ("GitHub Copilot" → "github-copilot").
pub(crate) fn nome_valido(s: &str) -> String {
    let mut n: String = s.trim().to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '-' }).collect();
    while n.contains("--") {
        n = n.replace("--", "-");
    }
    n.trim_matches('-').chars().take(64).collect()
}

/// Writes a manifest (local: command and arguments, with its environment; remote: address).
fn escrever_manifesto(dir: &std::path::Path, nome: &str, comando: Option<(&str, &[String])>, url: Option<&str>, ambiente: &BTreeMap<String, String>) -> Result<PathBuf, String> {
    let mut t = toml::Table::new();
    t.insert("nome".into(), toml::Value::String(nome.into()));
    match (comando, url) {
        (Some((c, a)), _) => {
            t.insert("comando".into(), toml::Value::String(c.into()));
            t.insert("argumentos".into(), toml::Value::Array(a.iter().map(|x| toml::Value::String(x.clone())).collect()));
        }
        (None, Some(u)) => {
            t.insert("url".into(), toml::Value::String(u.into()));
        }
        _ => return Err(tr("a connector needs a command or an address").into()),
    }
    if !ambiente.is_empty() {
        t.insert("ambiente".into(), toml::Value::Table(ambiente.iter().map(|(k, v)| (k.clone(), toml::Value::String(v.clone()))).collect()));
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let p = dir.join(format!("{nome}.toml"));
    std::fs::write(&p, toml::to_string(&t).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(p)
}

/// "Importar configuração": the usual MCP JSON (`{"mcpServers": {"name": {"command", "args", "env"} | {"url"}}}`),
/// one manifest per server in `dir`. Ok(names written).
pub fn importar(dir: &std::path::Path, texto: &str) -> Result<Vec<String>, String> {
    let v: Value = serde_json::from_str(texto).map_err(|e| e.to_string())?;
    let servidores = v["mcpServers"].as_object().or(v["servers"].as_object()).ok_or_else(|| tr("no “mcpServers” in this file").to_string())?;
    let mut feitos = Vec::new();
    for (n, s) in servidores {
        let nome = nome_valido(n);
        if nome.is_empty() {
            continue;
        }
        let args: Vec<String> = s["args"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(str::to_string)).collect();
        let env: BTreeMap<String, String> = s["env"].as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect();
        let r = match (s["command"].as_str(), s["url"].as_str()) {
            (Some(c), _) => escrever_manifesto(dir, &nome, Some((c, &args)), None, &env),
            (None, Some(u)) => escrever_manifesto(dir, &nome, None, Some(u), &env),
            _ => continue,
        };
        r?;
        feitos.push(nome);
    }
    Ok(feitos)
}

/// A tool as the page lists it: (full name, description, read only, destructive).
type Ferramenta = (String, String, bool, bool);

/// A third-party connector as the page shows it.
#[derive(Clone, Default)]
struct Terceiro {
    nome: String,
    caminho: PathBuf,
    comando: String,
    url: Option<String>,
    auth: &'static str,
    /// the handshake: Ok(tools) or Err(why)
    teste: Option<Result<Vec<Value>, String>>,
}

pub struct Ajustes {
    /// the list, or the page that adds a connector
    pub raiz: gtk::Stack,
    novo: Rc<crate::central_conector_novo::NovoConector>,
    numeros: [gtk::Label; 4],
    lista: gtk::Box,
    f_erro: gtk::Label,
    // the chosen connector
    p_icone: gtk::Box,
    p_nome: gtk::Label,
    p_estado: gtk::Label,
    p_chave: gtk::Switch,
    p_info: gtk::Box,
    p_titulo_ferr: gtk::Label,
    p_ferramentas: gtk::Box,
    p_filtros: Vec<gtk::ToggleButton>,
    p_quem: gtk::Box,
    p_testar: gtk::Button,
    p_registro: gtk::Button,
    p_remover: gtk::Button,
    p_remover_rotulo: gtk::Label,
    remover_confirma: Cell<bool>,
    normal: gtk::Box,
    diag: gtk::Box,
    d_texto: gtk::Label,
    d_acoes: gtk::Box,
    d_reg_sub: gtk::Label,
    d_codigo: gtk::Label,
    d_log: gtk::Label,
    d_afeta: gtk::Box,
    banner: gtk::Box,
    banner_texto: gtk::Label,
    cabecalho: gtk::Label,
    // data
    kalangod: RefCell<Vec<Value>>,
    permissoes: RefCell<BTreeMap<String, String>>,
    terceiros: RefCell<Vec<Terceiro>>,
    escolhido: RefCell<String>,
    /// the person chose a connector (until then a failed one is shown first)
    escolha_feita: Cell<bool>,
    botao_adicionar: gtk::Button,
    filtro: Cell<usize>,
    preenchendo: Cell<bool>,
}

impl Ajustes {
    pub fn new(shell: &Rc<Shell>, largura_janela: i32) -> Rc<Ajustes> {
        let compacta = largura_janela < 1480;
        let vista = gtk::Stack::new();
        vista.set_transition_type(gtk::StackTransitionType::Crossfade);
        vista.set_transition_duration(140);
        let raiz = caixa(Orientation::Horizontal, 0, "");
        raiz.set_vexpand(true);
        vista.add_named(&raiz, Some("lista"));
        let novo = crate::central_conector_novo::NovoConector::new(compacta);
        vista.add_named(&novo.raiz, Some("novo"));
        let corpo = caixa(Orientation::Vertical, 0, "central-corpo");
        corpo.set_hexpand(true);
        let h = rotulo(tr("Agent settings"), "central-titulo");
        h.set_xalign(0.0);
        corpo.append(&h);
        let sub = rotulo(tr("How the agents connect to apps and services."), "central-sub");
        sub.set_xalign(0.0);
        sub.set_margin_top(4);
        corpo.append(&sub);
        let abas = caixa(Orientation::Horizontal, 22, "ajustes-abas");
        abas.set_margin_top(12);
        // the tabs: the connectors (below) and the agents' general settings (agent_settings.rs), each with its own
        // side panel
        let abas_conteudo = gtk::Stack::new();
        abas_conteudo.set_transition_type(gtk::StackTransitionType::Crossfade);
        abas_conteudo.set_transition_duration(140);
        abas_conteudo.set_vexpand(true);
        abas_conteudo.set_hhomogeneous(false);
        let abas_lado = gtk::Stack::new();
        abas_lado.set_transition_type(gtk::StackTransitionType::Crossfade);
        abas_lado.set_transition_duration(140);
        abas_lado.set_hhomogeneous(false);
        let gerais = crate::agent_settings::AgentSettings::new();
        let mut botoes_abas: Vec<(&'static str, gtk::Button)> = Vec::new();
        for (id, nome) in [("geral", tr("General")), ("conectores", tr("Connectors (MCP)")), ("notificacoes", tr("Notifications")), ("avancado", tr("Advanced"))] {
            let b = gtk::Button::with_label(nome);
            b.add_css_class("ajustes-aba");
            if id == "conectores" {
                b.add_css_class("ativa");
            }
            crate::central::posicao_debug_sempre(&format!("ajustes aba {id}"), b.upcast_ref());
            abas.append(&b);
            botoes_abas.push((id, b));
        }
        for (id, b) in &botoes_abas {
            let (id, todos, c, l) = (*id, botoes_abas.iter().map(|(_, b)| b.clone()).collect::<Vec<_>>(), abas_conteudo.clone(), abas_lado.clone());
            b.connect_clicked(move |b| {
                for x in &todos {
                    x.remove_css_class("ativa");
                }
                b.add_css_class("ativa");
                c.set_visible_child_name(id);
                l.set_visible_child_name(id);
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug ajustes: aba {id}");
                }
            });
        }
        corpo.append(&abas);
        corpo.append(&caixa(Orientation::Horizontal, 0, "central-linha"));
        // the connectors' tab
        let con = caixa(Orientation::Vertical, 0, "");
        let topo = caixa(Orientation::Horizontal, 10, "");
        topo.set_margin_top(16);
        let exp = rotulo(tr("Connectors use the MCP protocol to give tools to the agents. Each one runs isolated and only offers what you allow."), "central-sub");
        exp.set_xalign(0.0);
        exp.set_wrap(true);
        exp.set_hexpand(true);
        topo.append(&exp);
        let importar_b = gtk::Button::new();
        importar_b.add_css_class("central-botao");
        // the same height and sides as "Adicionar conector" next to it (design)
        importar_b.add_css_class("ao-lado");
        let ic = caixa(Orientation::Horizontal, 8, "");
        ic.append(&ui_icon("baixar", "#E2E2E6", 14));
        ic.append(&rotulo(tr("Import configuration"), ""));
        importar_b.set_child(Some(&ic));
        importar_b.set_valign(Align::Center);
        topo.append(&importar_b);
        let adicionar = gtk::Button::new();
        adicionar.add_css_class("rotinas-nova");
        let ac = caixa(Orientation::Horizontal, 6, "");
        ac.append(&ui_icon("mais", "#1C1D24", 14));
        ac.append(&rotulo(tr("Add connector"), ""));
        adicionar.set_child(Some(&ac));
        adicionar.set_valign(Align::Center);
        topo.append(&adicionar);
        con.append(&topo);
        // an import that failed says why here
        let f_erro = rotulo("", "rotinas-erro");
        f_erro.set_xalign(0.0);
        f_erro.set_wrap(true);
        f_erro.set_visible(false);
        f_erro.set_margin_top(8);
        con.append(&f_erro);
        let stats = caixa(Orientation::Horizontal, 12, "");
        stats.set_homogeneous(true);
        stats.set_margin_top(14);
        let stat = |t: &str, c: &str| {
            let b = caixa(Orientation::Vertical, 4, "linha-stat");
            let n = rotulo("0", "linha-stat-numero");
            if !c.is_empty() {
                n.add_css_class(c);
            }
            n.set_xalign(0.0);
            b.append(&n);
            let l = rotulo(t, "central-agente-sub");
            l.set_xalign(0.0);
            b.append(&l);
            stats.append(&b);
            n
        };
        let numeros = [stat(tr("connectors"), ""), stat(tr("tools available"), ""), stat(tr("connected now"), "teal"), stat(tr("need attention"), "gold")];
        con.append(&stats);
        let banner = caixa(Orientation::Horizontal, 10, "ajustes-banner");
        banner.set_margin_top(14);
        banner.append(&ui_icon("alerta", "#EFA2A6", 15));
        let banner_texto = rotulo("", "central-limite-nome");
        banner_texto.set_hexpand(true);
        banner_texto.set_xalign(0.0);
        banner_texto.set_wrap(true);
        banner.append(&banner_texto);
        let resolver = gtk::Button::new();
        resolver.add_css_class("rotinas-sugestao-criar");
        let rcx = caixa(Orientation::Horizontal, 5, "");
        rcx.append(&estrela(11, "#C9C6F2"));
        rcx.append(&rotulo(tr("Solve it with the agent"), ""));
        resolver.set_child(Some(&rcx));
        resolver.set_valign(Align::Center);
        banner.append(&resolver);
        banner.set_visible(false);
        con.append(&banner);
        let lista = caixa(Orientation::Vertical, 8, "");
        lista.set_margin_top(14);
        let rolagem = gtk::ScrolledWindow::new();
        rolagem.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        rolagem.set_vexpand(true);
        rolagem.set_child(Some(&lista));
        con.append(&rolagem);
        let rodape = caixa(Orientation::Horizontal, 8, "");
        rodape.set_margin_top(12);
        rodape.append(&ui_icon("doc", "#8E8F99", 14));
        let home = glib::home_dir().display().to_string();
        let cfg = rotulo(&format!("{} {}", tr("Configuration in"), pasta().display().to_string().replacen(&home, "~", 1)), "central-agente-sub");
        cfg.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        cfg.set_max_width_chars(1);
        cfg.set_hexpand(true);
        cfg.set_xalign(0.0);
        rodape.append(&cfg);
        let editor = gtk::Button::new();
        editor.add_css_class("rotinas-texto");
        let ec = caixa(Orientation::Horizontal, 6, "");
        ec.append(&ui_icon("lapis", "#8E8F99", 13));
        ec.append(&rotulo(tr("Open the folder"), ""));
        editor.set_child(Some(&ec));
        rodape.append(&editor);
        con.append(&rodape);
        abas_conteudo.add_named(&con, Some("conectores"));
        abas_conteudo.add_named(&gerais.general.body, Some("geral"));
        abas_conteudo.add_named(&gerais.notifications.body, Some("notificacoes"));
        abas_conteudo.add_named(&gerais.advanced.body, Some("avancado"));
        abas_conteudo.set_visible_child_name("conectores");
        corpo.append(&abas_conteudo);
        raiz.append(&corpo);

        // the chosen connector
        let painel = caixa(Orientation::Vertical, 0, "central-painel");
        painel.set_size_request(if compacta { 340 } else { 474 }, -1);
        // fixed: what expands inside it must not widen it (GTK passes expand up to the ancestors)
        painel.set_hexpand(false);
        let cab = caixa(Orientation::Horizontal, 8, "");
        cab.append(&ui_icon("cabo", "#A5A1E2", 14));
        let cabecalho = rotulo(tr("CHOSEN CONNECTOR"), "central-painel-agente");
        cab.append(&cabecalho);
        painel.append(&cab);
        let qc = caixa(Orientation::Horizontal, 12, "");
        qc.set_margin_top(14);
        let p_icone = caixa(Orientation::Horizontal, 0, "ajustes-icone-grande");
        p_icone.set_size_request(44, 44);
        p_icone.set_hexpand(false);
        p_icone.set_valign(Align::Center);
        qc.append(&p_icone);
        let qcol = caixa(Orientation::Vertical, 2, "");
        qcol.set_hexpand(true);
        qcol.set_valign(Align::Center);
        let p_nome = rotulo("", "linha-titulo");
        p_nome.set_xalign(0.0);
        qcol.append(&p_nome);
        let p_estado = rotulo("", "central-agente-sub");
        p_estado.set_xalign(0.0);
        p_estado.set_ellipsize(gtk::pango::EllipsizeMode::End);
        p_estado.set_max_width_chars(1);
        p_estado.set_hexpand(true);
        qcol.append(&p_estado);
        qc.append(&qcol);
        let p_chave = gtk::Switch::new();
        p_chave.add_css_class("central-switch");
        p_chave.set_valign(Align::Center);
        qc.append(&p_chave);
        painel.append(&qc);
        let normal = caixa(Orientation::Vertical, 0, "");
        normal.set_vexpand(true);
        painel.append(&normal);
        let p_info = caixa(Orientation::Vertical, 6, "central-cartao");
        p_info.add_css_class("memoria-info");
        p_info.set_margin_top(14);
        normal.append(&p_info);
        let fcab = caixa(Orientation::Horizontal, 8, "");
        fcab.set_margin_top(18);
        let p_titulo_ferr = rotulo("", "central-painel-secao");
        p_titulo_ferr.set_hexpand(true);
        p_titulo_ferr.set_xalign(0.0);
        fcab.append(&p_titulo_ferr);
        let seg = caixa(Orientation::Horizontal, 0, "uso-segmentos");
        let mut p_filtros: Vec<gtk::ToggleButton> = Vec::new();
        for f in [tr("All"), tr("Read"), tr("Write")] {
            let b = gtk::ToggleButton::with_label(f);
            b.add_css_class("uso-segmento");
            if let Some(x) = p_filtros.first() {
                b.set_group(Some(x));
            }
            seg.append(&b);
            p_filtros.push(b);
        }
        fcab.append(&seg);
        normal.append(&fcab);
        let p_ferramentas = caixa(Orientation::Vertical, 0, "central-cartao");
        p_ferramentas.set_margin_top(10);
        let frol = gtk::ScrolledWindow::new();
        frol.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        frol.set_vexpand(true);
        frol.set_child(Some(&p_ferramentas));
        normal.append(&frol);
        let qp = rotulo(tr("WHO CAN USE IT"), "central-painel-secao");
        qp.set_xalign(0.0);
        qp.set_margin_top(14);
        normal.append(&qp);
        let p_quem = caixa(Orientation::Horizontal, 6, "");
        p_quem.set_margin_top(8);
        normal.append(&p_quem);
        let aviso = caixa(Orientation::Horizontal, 8, "");
        aviso.set_margin_top(14);
        aviso.append(&ui_icon("escudo", "#8ED2C3", 14));
        let at = rotulo(tr("What a connector returns is treated as data, never as an order for the agent."), "central-agente-sub");
        at.set_xalign(0.0);
        at.set_wrap(true);
        at.set_max_width_chars(1);
        at.set_hexpand(true);
        aviso.append(&at);
        normal.append(&aviso);
        let bs = caixa(Orientation::Horizontal, 8, "");
        // on a small screen (the 340 px panel) the buttons share it by their own widths, with short names
        bs.set_homogeneous(!compacta);
        bs.set_margin_top(10);
        let botao = |glifo: &str, cor: &str, t: &str, classe: &str| {
            let b = gtk::Button::new();
            b.add_css_class("central-botao");
            if !classe.is_empty() {
                b.add_css_class(classe);
            }
            let c = caixa(Orientation::Horizontal, 6, "");
            c.set_halign(Align::Center);
            c.append(&ui_icon(glifo, cor, 14));
            let l = rotulo(t, "");
            c.append(&l);
            b.set_child(Some(&c));
            bs.append(&b);
            (b, l)
        };
        let (p_testar, _) = botao("reiniciar", "#E2E2E6", if compacta { tr("Test") } else { tr("Test connection") }, "");
        let (p_registro, _) = botao("terminal", "#E2E2E6", if compacta { tr("Log") } else { tr("See log") }, "");
        if compacta {
            p_testar.set_tooltip_text(Some(tr("Test connection")));
            p_registro.set_tooltip_text(Some(tr("See log")));
            for b in [&p_testar, &p_registro] {
                b.set_hexpand(true);
            }
        }
        let (p_remover, p_remover_rotulo) = botao("lixeira", "#EFA2A6", tr("Remove"), "perigo");
        normal.append(&bs);
        // a third-party connector that failed: what happened (in plain words, with the fix), its own output, what it
        // affects
        let diag = caixa(Orientation::Vertical, 0, "");
        diag.set_vexpand(true);
        diag.set_visible(false);
        let oq = caixa(Orientation::Vertical, 8, "ajustes-oque");
        oq.set_margin_top(14);
        let oqc = caixa(Orientation::Horizontal, 7, "");
        oqc.append(&estrela(12, "#A5A1E2"));
        oqc.append(&rotulo(tr("WHAT HAPPENED"), "central-painel-agente"));
        oq.append(&oqc);
        let d_texto = rotulo("", "central-limite-nome");
        d_texto.set_xalign(0.0);
        d_texto.set_wrap(true);
        oq.append(&d_texto);
        let d_acoes = caixa(Orientation::Horizontal, 8, "");
        oq.append(&d_acoes);
        diag.append(&oq);
        let reg = caixa(Orientation::Vertical, 6, "ajustes-registro");
        reg.set_margin_top(12);
        let rc = caixa(Orientation::Horizontal, 8, "");
        rc.append(&ui_icon("terminal", "#C9CAD1", 14));
        rc.append(&rotulo(tr("Log"), "central-limite-nome"));
        let d_reg_sub = rotulo("", "central-agente-sub");
        rc.append(&d_reg_sub);
        let d_codigo = rotulo("", "linha-chip");
        d_codigo.add_css_class("negado");
        rc.append(&d_codigo);
        reg.append(&rc);
        let d_log = rotulo("", "ajustes-log");
        d_log.set_xalign(0.0);
        d_log.set_wrap(true);
        d_log.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        d_log.set_selectable(true);
        reg.append(&d_log);
        diag.append(&reg);
        let af = rotulo(tr("WHAT IT AFFECTS"), "central-painel-secao");
        af.set_xalign(0.0);
        af.set_margin_top(18);
        diag.append(&af);
        let d_afeta = caixa(Orientation::Vertical, 0, "central-cartao");
        d_afeta.set_margin_top(10);
        diag.append(&d_afeta);
        let dv = caixa(Orientation::Vertical, 0, "");
        dv.set_vexpand(true);
        diag.append(&dv);
        let dn = caixa(Orientation::Horizontal, 8, "");
        dn.append(&ui_icon("escudo", "#8ED2C3", 14));
        let dnt = rotulo(tr("A connector that fails never brings the agent down: it goes on without that connector's tools."), "central-agente-sub");
        dnt.set_xalign(0.0);
        dnt.set_wrap(true);
        dnt.set_max_width_chars(1);
        dnt.set_hexpand(true);
        dn.append(&dnt);
        diag.append(&dn);
        let dbs = caixa(Orientation::Horizontal, 8, "");
        dbs.set_homogeneous(true);
        dbs.set_margin_top(10);
        let dbotao = |glifo: &str, cor: &str, t: &str, classe: &str| {
            let b = gtk::Button::new();
            b.add_css_class("central-botao");
            if !classe.is_empty() {
                b.add_css_class(classe);
            }
            let c = caixa(Orientation::Horizontal, 6, "");
            c.set_halign(Align::Center);
            c.append(&ui_icon(glifo, cor, 14));
            c.append(&rotulo(t, ""));
            b.set_child(Some(&c));
            dbs.append(&b);
            b
        };
        let d_editar = dbotao("lapis", "#E2E2E6", tr("Edit configuration"), "");
        let d_desligar = dbotao("desligar", "#E2E2E6", tr("Turn connector off"), "");
        let d_remover = dbotao("lixeira", "#EFA2A6", tr("Remove"), "perigo");
        diag.append(&dbs);
        painel.append(&diag);
        abas_lado.add_named(&painel, Some("conectores"));
        for (id, lado) in [("geral", &gerais.general.side), ("notificacoes", &gerais.notifications.side), ("avancado", &gerais.advanced.side)] {
            lado.set_size_request(if compacta { 340 } else { 474 }, -1);
            lado.set_hexpand(false);
            abas_lado.add_named(lado, Some(id));
        }
        abas_lado.set_visible_child_name("conectores");
        raiz.append(&abas_lado);
        // (a narrow window: the side panel waits — the body below 520 px hides it, it comes back when the body
        // would keep 600 px with it)
        {
            let medidor = gtk::DrawingArea::new();
            medidor.set_can_target(false);
            medidor.set_hexpand(true);
            corpo.prepend(&medidor);
            let (lado, auto) = (abas_lado.clone(), Rc::new(Cell::new(false)));
            medidor.connect_resize(move |_, largura, _| {
                let (lado, auto) = (lado.clone(), auto.clone());
                glib::idle_add_local_once(move || {
                    let w = lado.width_request().max(lado.width()).max(340);
                    if lado.is_visible() && largura < 520 {
                        auto.set(true);
                        lado.set_visible(false);
                    } else if auto.get() && !lado.is_visible() && largura - w >= 600 {
                        auto.set(false);
                        lado.set_visible(true);
                    }
                });
            });
        }

        let a = Rc::new(Ajustes {
            raiz: vista, novo, numeros, lista, f_erro, p_icone, p_nome, p_estado, p_chave, p_info, p_titulo_ferr,
            p_ferramentas, p_filtros, p_quem, p_testar, p_registro, p_remover, p_remover_rotulo, remover_confirma: Cell::new(false),
            normal, diag, d_texto, d_acoes, d_reg_sub, d_codigo, d_log, d_afeta, banner, banner_texto, cabecalho,
            kalangod: RefCell::default(), permissoes: RefCell::default(), terceiros: RefCell::default(), escolhido: RefCell::new("arquivos".into()), escolha_feita: Cell::new(false), botao_adicionar: adicionar.clone(),
            filtro: Cell::new(0), preenchendo: Cell::new(false),
        });
        a.p_filtros[0].set_active(true);
        for (i, b) in a.p_filtros.iter().enumerate() {
            let w = Rc::downgrade(&a);
            b.connect_toggled(move |b| {
                if let Some(a) = w.upgrade().filter(|_| b.is_active()) {
                    a.filtro.set(i);
                    a.detalhe();
                }
            });
        }
        let w = Rc::downgrade(&a);
        adicionar.connect_clicked(move |_| {
            if let Some(a) = w.upgrade() {
                a.f_erro.set_visible(false);
                a.novo.limpar();
                a.raiz.set_visible_child_name("novo");
            }
        });
        let w = Rc::downgrade(&a);
        a.novo.ao_voltar(move |adicionado| {
            let Some(a) = w.upgrade() else { return };
            a.raiz.set_visible_child_name("lista");
            if let Some(n) = adicionado {
                a.escolha_feita.set(true);
                *a.escolhido.borrow_mut() = format!("ext:{n}");
            }
            a.recarregar();
        });
        let w = Rc::downgrade(&a);
        importar_b.connect_clicked(move |_| {
            let Some(a) = w.upgrade() else { return };
            let w = Rc::downgrade(&a);
            let filtros = vec![crate::arquivos::seletor::Filtro::de("JSON", &["*.json"])];
            crate::arquivos::seletor::abrir_arquivo(tr("Agents"), tr("Import an MCP configuration"), None, filtros, None, move |p| {
                let Some(a) = w.upgrade() else { return };
                let texto = std::fs::read_to_string(&p).unwrap_or_default();
                match importar(&pasta(), &texto) {
                    Ok(n) => {
                        if let Some(p) = n.first() {
                            *a.escolhido.borrow_mut() = format!("ext:{p}");
                        }
                        a.recarregar();
                    }
                    Err(e) => {
                        a.f_erro.set_text(&trf("Not imported: {why}", &[("why", e)]));
                        a.f_erro.set_visible(true);
                    }
                }
            });
        });
        editor.connect_clicked(move |b| {
            let _ = std::fs::create_dir_all(pasta());
            let l = gtk::FileLauncher::new(Some(&gio::File::for_path(pasta())));
            l.launch(b.root().and_downcast::<gtk::Window>().as_ref(), None::<&gio::Cancellable>, |_| {});
        });
        let w = Rc::downgrade(&a);
        a.p_chave.connect_state_set(move |_, on| {
            if let Some(a) = w.upgrade().filter(|a| !a.preenchendo.get()) {
                a.ligar(on);
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(&a);
        a.p_testar.connect_clicked(move |_| {
            if let Some(a) = w.upgrade() {
                a.recarregar();
            }
        });
        let w = Rc::downgrade(&a);
        a.p_registro.connect_clicked(move |b| {
            let Some(a) = w.upgrade() else { return };
            let esc = a.escolhido.borrow().clone();
            let Some(nome) = esc.strip_prefix("ext:") else { return };
            let estado_dir = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| glib::home_dir().join(".local/state"));
            let p = estado_dir.join(format!("kalango/conectores/{nome}.jsonl"));
            if !p.exists() {
                let _ = std::fs::create_dir_all(p.parent().unwrap());
                let _ = std::fs::write(&p, "");
            }
            let l = gtk::FileLauncher::new(Some(&gio::File::for_path(&p)));
            l.launch(b.root().and_downcast::<gtk::Window>().as_ref(), None::<&gio::Cancellable>, |_| {});
        });
        let w = Rc::downgrade(&a);
        d_editar.connect_clicked(move |b| {
            let Some(a) = w.upgrade() else { return };
            let esc = a.escolhido.borrow().clone();
            if let Some(nome) = esc.strip_prefix("ext:") {
                let l = gtk::FileLauncher::new(Some(&gio::File::for_path(pasta().join(format!("{nome}.toml")))));
                l.launch(b.root().and_downcast::<gtk::Window>().as_ref(), None::<&gio::Cancellable>, |_| {});
            }
        });
        let w = Rc::downgrade(&a);
        d_desligar.connect_clicked(move |_| {
            if let Some(a) = w.upgrade() {
                a.ligar(false);
            }
        });
        let pr = a.p_remover.clone();
        d_remover.connect_clicked(move |_| pr.emit_clicked());
        let (w, sh) = (Rc::downgrade(&a), Rc::downgrade(shell));
        resolver.connect_clicked(move |_| {
            let (Some(a), Some(s)) = (w.upgrade(), sh.upgrade()) else { return };
            let falhas: Vec<String> = a.terceiros.borrow().iter().filter_map(|t| match &t.teste {
                Some(Err(e)) => Some(format!("{} ({}): {}", t.nome, t.comando.clone() + t.url.as_deref().unwrap_or(""), e)),
                _ => None,
            }).collect();
            s.right_bar().ask_agent(&format!("{}\n{}", tr("These MCP connectors did not start; tell me what is wrong and how to fix it, in plain words:"), falhas.join("\n")));
        });
        let w = Rc::downgrade(&a);
        a.p_remover.connect_clicked(move |_| {
            let Some(a) = w.upgrade() else { return };
            let esc = a.escolhido.borrow().clone();
            let Some(nome) = esc.strip_prefix("ext:") else { return };
            if !a.remover_confirma.replace(true) {
                a.p_remover_rotulo.set_text(tr("Click again to remove"));
                let w = Rc::downgrade(&a);
                glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
                    if let Some(a) = w.upgrade().filter(|a| a.remover_confirma.replace(false)) {
                        a.p_remover_rotulo.set_text(tr("Remove"));
                    }
                });
                return;
            }
            a.remover_confirma.set(false);
            a.p_remover_rotulo.set_text(tr("Remove"));
            let _ = std::fs::remove_file(pasta().join(format!("{nome}.toml")));
            *a.escolhido.borrow_mut() = "arquivos".into();
            a.recarregar();
        });
        a
    }

    fn ligar(self: &Rc<Self>, on: bool) {
        let esc = self.escolhido.borrow().clone();
        if let Some(nome) = esc.strip_prefix("ext:") {
            set_ligado(nome, on);
            self.render();
            return;
        }
        // built in: its tools to "never", or back to their defaults
        let Some((_, _, _, prefixos)) = EMBUTIDOS.iter().find(|e| e.0 == esc) else { return };
        let ferramentas: Vec<String> = self.kalangod.borrow().iter().filter_map(|t| t["name"].as_str()).filter(|n| prefixos.iter().any(|p| n.starts_with(p))).map(str::to_string).collect();
        let bin = crate::voz::agent_binary();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            for t in ferramentas {
                let _ = Command::new(&bin).args(["permissao", &t, if on { "padrao" } else { "nunca" }]).status();
            }
        }, move |_| {
            if let Some(a) = w.upgrade() {
                a.recarregar();
            }
        });
    }

    /// Reads everything again (kalangod's tools, the agent's permissions, each third-party connector's handshake).
    pub fn recarregar(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        let bin = crate::voz::agent_binary();
        crate::lateral::em_segundo_plano(move || {
            let kalangod: Vec<Value> = crate::servicos::kalangod_ferramentas().unwrap_or_default().into_iter()
                .filter(|t| !INTERNAS.contains(&t["name"].as_str().unwrap_or(""))).collect();
            let perms: BTreeMap<String, String> = Command::new(&bin).arg("permissoes").output().ok().and_then(|o| serde_json::from_slice::<Vec<Value>>(&o.stdout).ok())
                .unwrap_or_default().iter().filter_map(|t| Some((t["nome"].as_str()?.to_string(), t["permissao"].as_str()?.to_string()))).collect();
            let mut terceiros: Vec<Terceiro> = std::fs::read_dir(pasta()).into_iter().flatten().filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "toml")).filter_map(|p| {
                    let t: toml::Table = std::fs::read_to_string(&p).ok()?.parse().ok()?;
                    let url = t.get("url").and_then(|v| v.as_str()).map(str::to_string);
                    let args: Vec<String> = t.get("argumentos").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
                    Some(Terceiro {
                        nome: p.file_stem()?.to_string_lossy().into_owned(), caminho: p.clone(),
                        comando: format!("{} {}", t.get("comando").and_then(|v| v.as_str()).unwrap_or(""), args.join(" ")).trim().to_string(),
                        auth: if t.get("token").is_some() { "token" } else if url.is_some() { "oauth" } else { "nenhuma" },
                        url, teste: None,
                    })
                }).collect();
            terceiros.sort_by(|a, b| a.nome.cmp(&b.nome));
            for t in &mut terceiros {
                t.teste = Some(testar(&t.caminho));
            }
            (kalangod, perms, terceiros)
        }, move |(k, p, t)| {
            let Some(a) = w.upgrade() else { return };
            *a.kalangod.borrow_mut() = k;
            *a.permissoes.borrow_mut() = p;
            *a.terceiros.borrow_mut() = t;
            // what needs attention comes first, until the person picks something
            if !a.escolha_feita.get() {
                let falha = a.terceiros.borrow().iter().find(|t| matches!(&t.teste, Some(Err(_)))).map(|t| format!("ext:{}", t.nome));
                if let Some(f) = falha {
                    *a.escolhido.borrow_mut() = f;
                }
            }
            a.render();
        });
    }

    /// The permission in force for a tool (built in: the agent's report; third party: its own setting, its column,
    /// else free when it only reads).
    fn permissao(&self, nome: &str, leitura: bool, destrutiva: bool) -> String {
        if let Some(p) = self.permissoes.borrow().get(nome) {
            return p.clone();
        }
        let cfg: Value = std::fs::read(glib::user_config_dir().join("kalango/agente-permissoes.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({}));
        if let Some(p) = cfg["ferramentas"][nome].as_str() {
            return p.to_string();
        }
        let coluna = if destrutiva { "apagar" } else if leitura { "ler" } else { "escrever" };
        cfg["categorias"][coluna].as_str().map(str::to_string).unwrap_or_else(|| if leitura { "livre".into() } else { "perguntar".into() })
    }

    fn render(self: &Rc<Self>) {
        let k = self.kalangod.borrow().clone();
        let ts = self.terceiros.borrow().clone();
        let est = estado();
        while let Some(x) = self.lista.first_child() {
            self.lista.remove(&x);
        }
        let mut total_ferr = 0;
        let mut conectados = 0;
        let mut atencao = 0;
        // built in
        for (id, nome, glifo, prefixos) in EMBUTIDOS {
            let ferr: Vec<&Value> = k.iter().filter(|t| t["name"].as_str().is_some_and(|n| prefixos.iter().any(|p| n.starts_with(p)))).collect();
            if ferr.is_empty() {
                continue;
            }
            let ligado = ferr.iter().any(|t| self.permissao(t["name"].as_str().unwrap_or(""), false, false) != "nunca");
            total_ferr += ferr.len();
            if ligado {
                conectados += 1;
            }
            let estado_t = if ligado { tr("connected").to_string() } else { tr("off").to_string() };
            let n_f = if ferr.len() == 1 { tr("1 tool").to_string() } else { trf("{n} tools", &[("n", ferr.len().to_string())]) };
            let quem = if ligado { tr("Agent") } else { tr("no agent") };
            self.linha(id, tr(nome), glifo, tr("built in · local"), if ligado { "ok" } else { "desligado" }, &estado_t, &format!("{n_f}  ·  {quem}"), ligado, None);
        }
        // third party
        for t in &ts {
            let ligado = est[&t.nome]["ligado"].as_bool().unwrap_or(true);
            let tipo = if t.url.is_some() { if t.auth == "token" { tr("remote · HTTP · token") } else { tr("remote · HTTP · OAuth") } } else { tr("local · command") };
            let (classe, estado_t, extra, acao): (&str, String, String, Option<&str>) = match &t.teste {
                Some(Ok(f)) => {
                    total_ferr += f.len();
                    if ligado {
                        conectados += 1;
                    }
                    let n_f = if f.len() == 1 { tr("1 tool").to_string() } else { trf("{n} tools", &[("n", f.len().to_string())]) };
                    (if ligado { "ok" } else { "desligado" }, if ligado { tr("connected").to_string() } else { tr("off").to_string() },
                        format!("{n_f}  ·  {}", if ligado { tr("Agent") } else { tr("no agent") }), None)
                }
                Some(Err(e)) => {
                    atencao += 1;
                    let sessao = t.url.is_some() && (e.contains("OAuth") || e.contains("entr") || e.contains("401") || e.contains("token"));
                    (if sessao { "atencao" } else { "erro" },
                        if sessao { tr("not signed in: sign in").to_string() } else { trf("did not start: {why}", &[("why", e.lines().next().unwrap_or("").chars().take(70).collect::<String>())]) },
                        tr("no tools").to_string(), Some(if sessao { "entrar" } else { "registro" }))
                }
                None => ("", tr("testing…").to_string(), String::new(), None),
            };
            self.linha(&format!("ext:{}", t.nome), &t.nome, if t.url.is_some() { "globo" } else { "terminal" }, tipo, classe, &estado_t, &extra, ligado, acao);
        }
        // what failed, said above the list
        let falhou = ts.iter().filter(|t| matches!(&t.teste, Some(Err(_)))).count();
        let entrar = ts.iter().filter(|t| t.url.is_some() && matches!(&t.teste, Some(Err(e)) if e.contains("OAuth") || e.contains("401"))).count();
        let ok_n = conectados;
        if falhou > 0 {
            let nao = falhou - entrar;
            let mut partes = Vec::new();
            if nao > 0 {
                partes.push(if nao == 1 { tr("1 connector did not start").to_string() } else { trf("{n} connectors did not start", &[("n", nao.to_string())]) });
            }
            if entrar > 0 {
                partes.push(if entrar == 1 { tr("1 needs to sign in").to_string() } else { trf("{n} need to sign in", &[("n", entrar.to_string())]) });
            }
            self.banner_texto.set_text(&format!("{}. {}", partes.join(&format!(" {} ", tr("and"))),
                if ok_n == 1 { tr("The 1 connected keeps working.").to_string() } else { trf("The {n} connected keep working.", &[("n", ok_n.to_string())]) }));
        }
        self.banner.set_visible(falhou > 0);
        let n_con = EMBUTIDOS.iter().filter(|(_, _, _, p)| k.iter().any(|t| t["name"].as_str().is_some_and(|n| p.iter().any(|x| n.starts_with(x))))).count() + ts.len();
        self.numeros[0].set_text(&n_con.to_string());
        self.numeros[1].set_text(&total_ferr.to_string());
        self.numeros[2].set_text(&conectados.to_string());
        self.numeros[3].set_text(&atencao.to_string());
        self.detalhe();
        crate::central::posicao_debug("botao adicionar conector", self.botao_adicionar.upcast_ref());
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            let ext: Vec<String> = ts.iter().map(|t| format!("{}={}", t.nome, match &t.teste { Some(Ok(f)) => format!("ok:{}", f.len()), Some(Err(_)) => "erro".into(), None => "?".into() })).collect();
            eprintln!("debug central: ajustes ({n_con} conectores, {total_ferr} ferramentas, {conectados} conectados, {atencao} atenção; {})", ext.join(" "));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn linha(self: &Rc<Self>, id: &str, nome: &str, glifo: &str, tipo: &str, classe: &str, estado_t: &str, extra: &str, ligado: bool, acao: Option<&'static str>) {
        let card = caixa(Orientation::Horizontal, 14, "rotinas-item");
        if *self.escolhido.borrow() == id {
            card.add_css_class("escolhido");
        }
        if !ligado {
            card.add_css_class("pausada");
        }
        let ic = caixa(Orientation::Horizontal, 0, "rotinas-icone");
        ic.set_size_request(36, 36);
        ic.set_hexpand(false);
        ic.set_valign(Align::Center);
        let g = ui_icon(glifo, "#C9CAD1", 16);
        g.set_hexpand(true);
        g.set_halign(Align::Center);
        ic.append(&g);
        card.append(&ic);
        let col = caixa(Orientation::Vertical, 4, "");
        col.set_hexpand(true);
        col.set_valign(Align::Center);
        let l1 = caixa(Orientation::Horizontal, 8, "");
        l1.append(&rotulo(nome, "rotinas-nome"));
        let chip = rotulo(tipo, "ajustes-chip");
        chip.set_valign(Align::Center);
        l1.append(&chip);
        col.append(&l1);
        let l2 = caixa(Orientation::Horizontal, 6, "");
        let ponto = caixa(Orientation::Vertical, 0, "ajustes-ponto");
        if !classe.is_empty() {
            ponto.add_css_class(classe);
        }
        ponto.set_valign(Align::Center);
        l2.append(&ponto);
        let e = rotulo(estado_t, "ajustes-estado");
        if !classe.is_empty() {
            e.add_css_class(classe);
        }
        e.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l2.append(&e);
        if !extra.is_empty() {
            l2.append(&rotulo(&format!("·  {extra}"), "central-agente-sub"));
        }
        col.append(&l2);
        card.append(&col);
        if let Some(a) = acao {
            let b = gtk::Button::with_label(if a == "entrar" { tr("Sign in") } else { tr("See log") });
            b.add_css_class(if a == "entrar" { "linha-revisar" } else { "ajustes-erro-botao" });
            b.set_valign(Align::Center);
            let nome_c = id.trim_start_matches("ext:").to_string();
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                let Some(a_) = w.upgrade() else { return };
                *a_.escolhido.borrow_mut() = format!("ext:{nome_c}");
                if a == "entrar" {
                    let p = pasta().join(format!("{nome_c}.toml"));
                    let w = Rc::downgrade(&a_);
                    crate::lateral::em_segundo_plano(move || Command::new(kalangod_bin()).args(["conector", "entrar"]).arg(p).status().map(|s| s.success()).unwrap_or(false),
                        move |_| if let Some(a) = w.upgrade() { a.recarregar() });
                } else {
                    a_.p_registro.emit_clicked();
                }
            });
            card.append(&b);
        }
        let sw = gtk::Switch::new();
        sw.add_css_class("central-switch");
        sw.set_active(ligado);
        sw.set_valign(Align::Center);
        let (w, idc) = (Rc::downgrade(self), id.to_string());
        sw.connect_state_set(move |_, on| {
            if let Some(a) = w.upgrade().filter(|a| !a.preenchendo.get()) {
                *a.escolhido.borrow_mut() = idc.clone();
                a.ligar(on);
            }
            glib::Propagation::Proceed
        });
        card.append(&sw);
        let clique = gtk::GestureClick::new();
        let (w, idc) = (Rc::downgrade(self), id.to_string());
        clique.connect_released(move |_, _, _, _| {
            if let Some(a) = w.upgrade() {
                a.escolha_feita.set(true);
                *a.escolhido.borrow_mut() = idc.clone();
                let w = Rc::downgrade(&a);
                glib::idle_add_local_once(move || if let Some(a) = w.upgrade() { a.render() });
            }
        });
        card.add_controller(clique);
        crate::central::posicao_debug(&format!("conector {id}"), card.upcast_ref());
        self.lista.append(&card);
    }

    fn detalhe(self: &Rc<Self>) {
        self.preenchendo.set(true);
        let esc = self.escolhido.borrow().clone();
        let k = self.kalangod.borrow().clone();
        for c in [&self.p_icone, &self.p_info, &self.p_ferramentas, &self.p_quem] {
            while let Some(x) = c.first_child() {
                c.remove(&x);
            }
        }
        let info = |nome: &str, valor: &str, classe: &str| {
            let l = caixa(Orientation::Horizontal, 8, "");
            let n = rotulo(nome, "central-agente-sub");
            n.set_hexpand(true);
            n.set_xalign(0.0);
            l.append(&n);
            let v = rotulo(valor, "memoria-info-valor");
            if !classe.is_empty() {
                v.add_css_class(classe);
            }
            v.set_ellipsize(gtk::pango::EllipsizeMode::Start);
            // (a narrow panel: the value cuts at its start rather than widening it)
            v.set_max_width_chars(24);
            l.append(&v);
            self.p_info.append(&l);
        };
        let (glifo, nome, estado_t, ligado, ferramentas, externo): (&str, String, String, bool, Vec<Ferramenta>, bool) = match esc.strip_prefix("ext:") {
            Some(n) => {
                let t = self.terceiros.borrow().iter().find(|t| t.nome == n).cloned().unwrap_or_default();
                let ligado = estado()[n]["ligado"].as_bool().unwrap_or(true);
                info(if t.url.is_some() { tr("Address") } else { tr("Command") }, t.url.as_deref().unwrap_or(&t.comando), "mono");
                info(tr("Transport"), if t.url.is_some() { tr("HTTP (streamable)") } else { tr("stdio, isolated by kalangod") }, "");
                info(tr("Authentication"), match t.auth { "oauth" => tr("OAuth"), "token" => tr("token in the vault"), _ => tr("none") }, "");
                let (estado_t, ferr) = match &t.teste {
                    Some(Ok(f)) => (if ligado { tr("connected").to_string() } else { tr("off").to_string() },
                        f.iter().map(|x| (format!("{n}.{}", x["nome"].as_str().unwrap_or("")), x["descricao"].as_str().unwrap_or("").to_string(),
                            x["leitura"] == true, x["destrutiva"] == true)).collect()),
                    Some(Err(e)) => (trf("did not start: {why}", &[("why", e.lines().next().unwrap_or("").to_string())]), vec![]),
                    None => (tr("testing…").to_string(), vec![]),
                };
                info(tr("Offers"), &if ferr.len() == 1 { tr("1 tool").to_string() } else { trf("{n} tools", &[("n", ferr.len().to_string())]) }, "");
                (if t.url.is_some() { "globo" } else { "terminal" }, n.to_string(), estado_t, ligado, ferr, true)
            }
            None => {
                let (_, nome, glifo, prefixos) = EMBUTIDOS.iter().find(|e| e.0 == esc).copied().unwrap_or(EMBUTIDOS[0]);
                let ferr: Vec<Ferramenta> = k.iter().filter(|t| t["name"].as_str().is_some_and(|n| prefixos.iter().any(|p| n.starts_with(p))))
                    .map(|t| (t["name"].as_str().unwrap_or("").to_string(), t["description"].as_str().unwrap_or("").split(" [").next().unwrap_or("").to_string(),
                        t["annotations"]["readOnlyHint"] == true, t["annotations"]["destructiveHint"] == true)).collect();
                let ligado = ferr.iter().any(|(n, _, l, d)| self.permissao(n, *l, *d) != "nunca");
                info(tr("Address"), tr("kalangod (the system's socket)"), "mono");
                info(tr("Transport"), tr("local MCP"), "");
                info(tr("Authentication"), tr("by the process (its user)"), "");
                info(tr("Offers"), &trf("{n} tools", &[("n", ferr.len().to_string())]), "");
                (glifo, tr(nome).to_string(), if ligado { tr("connected").to_string() } else { tr("off").to_string() }, ligado, ferr, false)
            }
        };
        // a third-party connector that failed: the diagnosis instead of the tools
        let falha = esc.strip_prefix("ext:").and_then(|n| self.terceiros.borrow().iter().find(|t| t.nome == n).and_then(|t| match &t.teste {
            Some(Err(e)) => Some((t.clone(), e.clone())),
            _ => None,
        }));
        self.normal.set_visible(falha.is_none());
        self.diag.set_visible(falha.is_some());
        self.cabecalho.set_text(if falha.is_some() { tr("DIAGNOSIS") } else { tr("CHOSEN CONNECTOR") });
        if let Some((t, e)) = &falha {
            self.mostrar_diagnostico(t, e);
        }
        let g = ui_icon(if falha.is_some() { "alerta" } else { glifo }, if falha.is_some() { "#EFA2A6" } else { "#8ED2C3" }, 20);
        g.set_hexpand(true);
        g.set_halign(Align::Center);
        self.p_icone.append(&g);
        self.p_nome.set_text(&nome);
        self.p_estado.set_text(&estado_t);
        self.p_chave.set_active(ligado);
        self.p_titulo_ferr.set_text(&trf("TOOLS ({n})", &[("n", ferramentas.len().to_string())]));
        let filtro = self.filtro.get();
        for (n, d, leitura, destrutiva) in ferramentas.iter().filter(|(_, _, l, _)| match filtro { 1 => *l, 2 => !*l, _ => true }) {
            let l = caixa(Orientation::Horizontal, 8, "ajustes-ferramenta");
            let col = caixa(Orientation::Vertical, 1, "");
            col.set_hexpand(true);
            let nn = rotulo(n.rsplit_once('.').map(|x| if externo { x.1 } else { n.as_str() }).unwrap_or(n), "ajustes-ferramenta-nome");
            nn.set_xalign(0.0);
            col.append(&nn);
            let dd = rotulo(d, "central-agente-sub");
            dd.set_xalign(0.0);
            dd.set_ellipsize(gtk::pango::EllipsizeMode::End);
            dd.set_max_width_chars(1);
            dd.set_hexpand(true);
            col.append(&dd);
            l.append(&col);
            let tipo = rotulo(if *destrutiva { tr("destructive (kind)") } else if *leitura { tr("reading (kind)") } else { tr("writes (kind)") }, "linha-chip");
            tipo.add_css_class(if *destrutiva { "negado" } else if *leitura { "leitura" } else { "espera" });
            tipo.set_valign(Align::Center);
            l.append(&tipo);
            let p = self.permissao(n, *leitura, *destrutiva);
            let pb = gtk::Button::with_label(match p.as_str() { "livre" => tr("free"), "nunca" => tr("never"), _ => tr("ask") });
            pb.add_css_class("central-celula");
            pb.add_css_class(&p);
            pb.set_valign(Align::Center);
            pb.set_tooltip_text(Some(tr("Click to change")));
            let (w, n2) = (Rc::downgrade(self), n.clone());
            pb.connect_clicked(move |_| {
                let prox = match p.as_str() { "livre" => "perguntar", "perguntar" => "nunca", _ => "livre" };
                let bin = crate::voz::agent_binary();
                let w = w.clone();
                let n3 = n2.clone();
                crate::lateral::em_segundo_plano(move || Command::new(&bin).args(["permissao", &n3, prox]).status().map(|s| s.success()).unwrap_or(false),
                    move |_| if let Some(a) = w.upgrade() { a.recarregar() });
            });
            l.append(&pb);
            self.p_ferramentas.append(&l);
        }
        if ferramentas.is_empty() {
            let v = rotulo(tr("No tools."), "central-agente-sub");
            v.set_xalign(0.0);
            v.set_margin_top(6);
            v.set_margin_bottom(6);
            self.p_ferramentas.append(&v);
        }
        let quem = caixa(Orientation::Horizontal, 5, "ajustes-quem");
        if ligado {
            quem.add_css_class("ativo");
        }
        quem.append(&estrela(11, if ligado { "#1C1D24" } else { "#8E8F99" }));
        quem.append(&rotulo(tr("Agent"), ""));
        self.p_quem.append(&quem);
        self.p_registro.set_sensitive(externo);
        self.p_remover.set_sensitive(externo);
        self.p_testar.set_sensitive(true);
        self.preenchendo.set(false);
    }
}

impl Ajustes {
    fn mostrar_diagnostico(self: &Rc<Self>, t: &Terceiro, erro: &str) {
        let estado_dir = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| glib::home_dir().join(".local/state"));
        let log = std::fs::read_to_string(estado_dir.join(format!("kalango/conectores/{}.stderr.log", t.nome))).unwrap_or_default();
        let manifesto: toml::Table = std::fs::read_to_string(&t.caminho).ok().and_then(|x| x.parse().ok()).unwrap_or_default();
        let rede = manifesto.get("rede").and_then(|v| v.as_bool()).unwrap_or(false);
        let (texto, correcao) = diagnostico(erro, &log, &t.comando, rede, t.url.is_some());
        self.d_texto.set_text(&texto);
        while let Some(x) = self.d_acoes.first_child() {
            self.d_acoes.remove(&x);
        }
        let botao = |glifo: &str, rotulo_t: &str, principal: bool| {
            let b = gtk::Button::new();
            b.add_css_class(if principal { "rotinas-nova" } else { "linha-acao" });
            let c = caixa(Orientation::Horizontal, 6, "");
            c.append(&ui_icon(glifo, if principal { "#1C1D24" } else { "#E2E2E6" }, 13));
            c.append(&rotulo(rotulo_t, ""));
            b.set_child(Some(&c));
            self.d_acoes.append(&b);
            b
        };
        if let Some(c) = correcao {
            let rotulo_c = match &c {
                Correcao::Instalar(app, _) => trf("Install {app}", &[("app", app.to_string())]),
                Correcao::PermitirRede => tr("Allow the network").to_string(),
                Correcao::Entrar => tr("Sign in").to_string(),
            };
            let b = botao(match &c { Correcao::Instalar(..) => "baixar", Correcao::PermitirRede => "globo", Correcao::Entrar => "cadeado" }, &rotulo_c, true);
            crate::central::posicao_debug("correcao", b.upcast_ref());
            let (w, caminho) = (Rc::downgrade(self), t.caminho.clone());
            b.connect_clicked(move |b| {
                b.set_sensitive(false);
                let c = c.clone();
                let caminho = caminho.clone();
                let w = w.clone();
                crate::lateral::em_segundo_plano(move || match c {
                    // a system change: kalangod asks for approval on its screen and makes a restore point
                    Correcao::Instalar(_, pacotes) => crate::servicos::kalangod_chamar("pacotes.instalar", json!({"nomes": pacotes})).map(|_| ()),
                    Correcao::PermitirRede => {
                        let mut m: toml::Table = std::fs::read_to_string(&caminho).ok().and_then(|x| x.parse().ok()).unwrap_or_default();
                        m.insert("rede".into(), toml::Value::Boolean(true));
                        toml::to_string(&m).map_err(|e| e.to_string()).and_then(|x| std::fs::write(&caminho, x).map_err(|e| e.to_string()))
                    }
                    Correcao::Entrar => Command::new(kalangod_bin()).args(["conector", "entrar"]).arg(&caminho).status().map(|_| ()).map_err(|e| e.to_string()),
                }, move |r| {
                    if let Err(e) = r {
                        eprintln!("kalango-shell: correção do conector: {e}");
                    }
                    if let Some(a) = w.upgrade() {
                        a.recarregar();
                    }
                });
            });
        }
        let tentar = botao("reiniciar", tr("Try again"), false);
        let w = Rc::downgrade(self);
        tentar.connect_clicked(move |_| {
            if let Some(a) = w.upgrade() {
                a.recarregar();
            }
        });
        // its own output
        let codigo = erro.rsplit_once('(').and_then(|(_, c)| c.trim_end_matches(')').split_whitespace().last().map(str::to_string)).filter(|c| c.chars().all(|x| x.is_ascii_digit()));
        self.d_reg_sub.set_text(&format!("{} · stderr", t.nome));
        self.d_codigo.set_text(&codigo.as_ref().map(|c| trf("code {code}", &[("code", c.clone())])).unwrap_or_default());
        self.d_codigo.set_visible(codigo.is_some());
        let linhas: Vec<&str> = log.lines().collect();
        let k = linhas.len().saturating_sub(8);
        self.d_log.set_text(&if linhas.is_empty() { tr("(no output)").to_string() } else { linhas[k..].join("\n") });
        // what it affects: the routines that name it, and the connectors that keep working
        while let Some(x) = self.d_afeta.first_child() {
            self.d_afeta.remove(&x);
        }
        let linha_af = |glifo: &str, t1: &str, t2: &str, chip: &str, classe: &str| {
            let l = caixa(Orientation::Horizontal, 12, "memoria-chave");
            l.append(&ui_icon(glifo, "#C9CAD1", 15));
            let c = caixa(Orientation::Vertical, 1, "");
            c.set_hexpand(true);
            c.set_valign(Align::Center);
            let a1 = rotulo(t1, "central-limite-nome");
            a1.set_xalign(0.0);
            c.append(&a1);
            let a2 = rotulo(t2, "central-agente-sub");
            a2.set_xalign(0.0);
            c.append(&a2);
            l.append(&c);
            let ch = rotulo(chip, "linha-chip");
            if !classe.is_empty() {
                ch.add_css_class(classe);
            }
            ch.set_valign(Align::Center);
            l.append(&ch);
            self.d_afeta.append(&l);
        };
        let nome_l = t.nome.to_lowercase();
        for r in crate::rotinas::Rotinas::carregar().lista.iter().filter(|r| matches!(&r.acao, crate::rotinas::Acao::Pedido(p) if p.to_lowercase().contains(&nome_l))) {
            linha_af("raio", &trf("Routine “{name}”", &[("name", r.nome.clone())]), tr("runs without this connector's tools until it is back"), tr("waiting"), "espera");
        }
        let ok_n = self.terceiros.borrow().iter().filter(|x| matches!(&x.teste, Some(Ok(_)))).count()
            + EMBUTIDOS.iter().filter(|(_, _, _, p)| self.kalangod.borrow().iter().any(|t| t["name"].as_str().is_some_and(|n| p.iter().any(|x| n.starts_with(x))))).count();
        linha_af("check", tr("Connectors already connected"), &trf("the {n} keep working normally", &[("n", ok_n.to_string())]), tr("ok"), "reversivel");
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug central: diagnóstico {} ({texto})", t.nome);
        }
    }
}

/// What a fix does: install packages (through kalangod, with approval), allow the network to the connector, sign in.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Correcao {
    Instalar(&'static str, Vec<&'static str>),
    PermitirRede,
    Entrar,
}

/// The failure in plain words, and the fix when there is one — from the error, the connector's own output and its
/// manifest.
pub(crate) fn diagnostico(erro: &str, log: &str, comando: &str, rede: bool, remoto: bool) -> (String, Option<Correcao>) {
    let base = std::path::Path::new(comando.split_whitespace().next().unwrap_or("")).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let falta = log.contains("ENOENT") || log.contains("No such file") || log.contains("not found") || erro.contains("126") || erro.contains("127");
    if remoto && (erro.contains("OAuth") || erro.contains("401") || erro.contains("entr") || erro.contains("sign")) {
        return (tr("The service needs you to sign in (again). The connector's configuration is right.").to_string(), Some(Correcao::Entrar));
    }
    if falta {
        let pacote: Option<(&'static str, Vec<&'static str>)> = match base.as_str() {
            "npx" | "node" | "npm" => Some(("Node.js", vec!["nodejs", "npm"])),
            "python3" | "python" => Some(("Python", vec!["python3"])),
            "docker" => Some(("Docker", vec!["docker.io"])),
            "uvx" | "uv" => None,
            _ => None,
        };
        return match pacote {
            Some((nome, pacotes)) => (trf("The command {cmd} does not exist on this computer: {app} is missing. The connector and its configuration are right.", &[("cmd", base.clone()), ("app", nome.to_string())]),
                Some(Correcao::Instalar(nome, pacotes))),
            None => (trf("The command {cmd} does not exist on this computer. Check the path, or install the program that provides it.", &[("cmd", base.clone())]), None),
        };
    }
    if erro.contains("15 s") || erro.contains("15 s") {
        if !rede && matches!(base.as_str(), "npx" | "uvx" | "pipx" | "bunx" | "pnpx") {
            return (trf("{cmd} needs to download the package from the internet, but this connector runs without network. Allow the network to it, or install the package first.", &[("cmd", base.clone())]),
                Some(Correcao::PermitirRede));
        }
        return (tr("The program started but did not answer as an MCP server within 15 s. Check the command and its arguments.").to_string(), None);
    }
    if erro.contains("protegida") || erro.contains("protected") {
        return (tr("The configuration asks for a protected folder (keys, keyring or vault), which no connector reaches.").to_string(), None);
    }
    (erro.lines().next().unwrap_or(erro).to_string(), None)
}

/// The real handshake of a third-party connector (`kalangod conector testar`): Ok(its tools) or Err(why).
fn testar(manifesto: &std::path::Path) -> Result<Vec<Value>, String> {
    let o = Command::new(kalangod_bin()).args(["conector", "testar"]).arg(manifesto).output().map_err(|e| e.to_string())?;
    if !o.status.success() {
        let e = String::from_utf8_lossy(&o.stderr).trim().trim_start_matches("kalangod: ").to_string();
        return Err(if e.is_empty() { tr("the connector did not answer").to_string() } else { e });
    }
    let v: Value = serde_json::from_slice(&o.stdout).map_err(|e| e.to_string())?;
    Ok(v["ferramentas"].as_array().cloned().unwrap_or_default())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_import() {
        assert_eq!(nome_valido("GitHub Copilot!"), "github-copilot");
        let d = tempfile_dir();
        let n = importar(&d, r#"{"mcpServers": {"Fig Craft": {"command": "/usr/bin/figcraft", "args": ["mcp"], "env": {"A": "1"}}, "remoto": {"url": "https://x.example/mcp"}}}"#).unwrap();
        assert_eq!(n, vec!["fig-craft".to_string(), "remoto".to_string()]);
        let t: toml::Table = std::fs::read_to_string(d.join("fig-craft.toml")).unwrap().parse().unwrap();
        assert_eq!(t["comando"].as_str(), Some("/usr/bin/figcraft"));
        assert_eq!(t["argumentos"].as_array().unwrap()[0].as_str(), Some("mcp"));
        assert_eq!(t["ambiente"]["A"].as_str(), Some("1"));
        let r: toml::Table = std::fs::read_to_string(d.join("remoto.toml")).unwrap().parse().unwrap();
        assert_eq!(r["url"].as_str(), Some("https://x.example/mcp"));
        assert!(importar(&d, "{}").is_err());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn diagnosis_in_plain_words() {
        // npx missing: Node.js is installed through kalangod
        let (t, c) = diagnostico("the process ended before answering (code 126)", "kalangod __tarefa: /usr/bin/npx: ENOENT", "/usr/bin/npx -y x", false, false);
        assert!(t.contains("npx") && t.contains("Node.js"), "{t}");
        assert_eq!(c, Some(Correcao::Instalar("Node.js", vec!["nodejs", "npm"])));
        // npx present but without network: it cannot download the package
        let (t, c) = diagnostico("the connector did not answer initialize within 15 s", "", "/usr/bin/npx -y x", false, false);
        assert!(t.contains("npx"), "{t}");
        assert_eq!(c, Some(Correcao::PermitirRede));
        // another program that does not answer: no fix to offer
        assert_eq!(diagnostico("the connector did not answer initialize within 15 s", "", "/opt/x/servidor", true, false).1, None);
        // a remote one that needs to sign in
        assert_eq!(diagnostico("OAuth: 401", "", "", false, true).1, Some(Correcao::Entrar));
        // an unknown missing program: said, no package guessed
        let (t, c) = diagnostico("x (code 126)", "ENOENT", "/usr/bin/servidor-x", false, false);
        assert!(t.contains("servidor-x") && c.is_none(), "{t}");
    }

    fn tempfile_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("kalango-ajustes-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    }
}
