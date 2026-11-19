//! "Instalar agente (permissões antes de instalar)" (design "Instalar agente"): a third-party agent comes as a
//! manifest (`*.kalango-agente`, TOML: name, publisher, description, its instructions, when it runs, the autonomy it
//! suggests and the permissions it asks for — folders, programs, sites, the screen). Nothing installs before the
//! person sees — and can reduce — those permissions; the main button repeats the decision ("Instalar com estas
//! permissões"). "Experimentar em simulação" runs it for real once, without changing anything (`--simular`), and
//! shows what it would do. Installed, it is a routine that runs with exactly that scope (`--pastas`, `--programas`,
//! `--dominios`, the columns it does not use turned off, its autonomy).

use crate::conversa::ui_icon;
use crate::rotinas::{Acao, AgenteConfig, Gatilho, Rotinas};
use crate::superficies::{caixa, estrela, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use serde::Deserialize;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Quando {
    #[serde(default)]
    pub dias: Vec<u8>,
    #[serde(default = "dezoito")]
    pub hora: u8,
    #[serde(default)]
    pub minuto: u8,
}

fn dezoito() -> u8 {
    18
}

/// One permission the agent asks for: "ler", "escrever", "apagar" (with `pastas`), "programas" (with `programas`),
/// "rede" (with `dominios`) or "tela".
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Pedido {
    pub tipo: String,
    #[serde(default)]
    pub pastas: Vec<String>,
    #[serde(default)]
    pub programas: Vec<String>,
    #[serde(default)]
    pub dominios: Vec<String>,
    #[serde(default)]
    pub detalhe: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Manifesto {
    pub nome: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub editor: String,
    #[serde(default)]
    pub descricao: String,
    /// one line for the App Center's card
    #[serde(default)]
    pub resumo: String,
    /// the App Center's chip: "produtividade", "desenvolvimento", "criacao", "sistema", "jogos"
    #[serde(default)]
    pub categoria: String,
    pub instrucoes: String,
    #[serde(default)]
    pub autonomia: String,
    #[serde(default)]
    pub quando: Option<Quando>,
    #[serde(default)]
    pub permissoes: Vec<Pedido>,
}

/// A manifest read and checked: a name and instructions, known kinds of permission only, valid days and time.
pub fn read(path: &Path) -> Result<Manifesto, String> {
    let t = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut m: Manifesto = toml::from_str(&t).map_err(|e| trf("this agent's file is not valid: {error}", &[("error", e.message().to_string())]))?;
    if m.nome.trim().is_empty() || m.instrucoes.trim().is_empty() {
        return Err(tr("this agent's file has no name or no instructions").into());
    }
    m.permissoes.retain(|p| matches!(p.tipo.as_str(), "ler" | "escrever" | "apagar" | "programas" | "rede" | "tela"));
    if let Some(q) = m.quando.as_mut() {
        q.dias.retain(|d| (1..=7).contains(d));
        q.hora = q.hora.min(23);
        q.minuto = q.minuto.min(59);
    }
    if !matches!(m.autonomia.as_str(), "sempre" | "risco" | "autonomo") {
        m.autonomia = "risco".into();
    }
    Ok(m)
}

/// What an install keeps: the folders, programs and sites it may use, and the permission columns it runs without
/// (what it did not ask for, and what the person switched off).
pub fn scope(m: &Manifesto, ligadas: &[bool]) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let mut pastas: Vec<String> = Vec::new();
    let (mut programas, mut dominios) = (Vec::new(), Vec::new());
    let mut usa: Vec<&str> = Vec::new();
    for (p, on) in m.permissoes.iter().zip(ligadas.iter().copied().chain(std::iter::repeat(true))) {
        if !on {
            continue;
        }
        let coluna = match p.tipo.as_str() {
            "programas" => "terminal",
            "tela" => "tela",
            t => t,
        };
        usa.push(coluna);
        for x in &p.pastas {
            if !pastas.contains(x) {
                pastas.push(x.clone());
            }
        }
        programas.extend(p.programas.iter().cloned());
        dominios.extend(p.dominios.iter().cloned());
    }
    let desligadas = ["ler", "escrever", "apagar", "rede", "terminal", "tela", "gastar"].iter().filter(|c| !usa.contains(c)).map(|c| c.to_string()).collect();
    (pastas, programas, dominios, desligadas)
}

fn secao(t: &str, icone: &str) -> gtk::Box {
    let l = caixa(Orientation::Horizontal, 8, "");
    l.set_margin_top(18);
    l.set_margin_bottom(10);
    l.append(&ui_icon(icone, "#8E8F99", 13));
    l.append(&rotulo(t, "central-painel-secao"));
    l
}

pub struct InstallAgentPage {
    pub raiz: gtk::Box,
    corpo: gtk::Box,
    lado: gtk::Box,
    instalar: gtk::Button,
    simular: gtk::Button,
    resultado: gtk::Box,
    manifesto: RefCell<Option<(PathBuf, Manifesto)>>,
    ligadas: RefCell<Vec<bool>>,
    autonomia: RefCell<String>,
    instalando: Cell<bool>,
    /// the install button repeated under the simulation's result
    instalar_fim: RefCell<Option<gtk::Button>>,
    /// after installing: where to go (the routines page)
    pub ir: RefCell<Option<Box<dyn Fn(&str)>>>,
}

/// The progress bar a button holds (while installing).
fn f_bar(b: &glib::WeakRef<gtk::Button>) -> Option<gtk::LevelBar> {
    b.upgrade()?.child().and_downcast::<gtk::LevelBar>()
}

impl InstallAgentPage {
    pub fn new() -> Rc<InstallAgentPage> {
        let raiz = caixa(Orientation::Horizontal, 0, "novo-agente");
        raiz.set_vexpand(true);
        let corpo = caixa(Orientation::Vertical, 0, "novo-agente-esq");
        let rol = gtk::ScrolledWindow::new();
        rol.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        rol.set_hexpand(true);
        rol.set_child(Some(&corpo));
        raiz.append(&rol);
        let lado = caixa(Orientation::Vertical, 0, "novo-agente-lado");
        lado.set_size_request(380, -1);
        raiz.append(&lado);
        let instalar = gtk::Button::new();
        instalar.add_css_class("botao-principal");
        let simular = gtk::Button::new();
        simular.add_css_class("novo-agente-rascunho");
        let resultado = caixa(Orientation::Vertical, 6, "");
        crate::central::posicao_debug_sempre("instalar agente instalar", instalar.upcast_ref());
        crate::central::posicao_debug_sempre("instalar agente simular", simular.upcast_ref());
        let p = Rc::new(InstallAgentPage {
            raiz, corpo, lado, instalar: instalar.clone(), simular: simular.clone(), resultado, manifesto: RefCell::default(), ligadas: RefCell::default(),
            autonomia: RefCell::new("risco".into()), instalando: Cell::new(false), instalar_fim: RefCell::default(), ir: RefCell::default(),
        });
        let w = Rc::downgrade(&p);
        instalar.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.install();
            }
        });
        let w = Rc::downgrade(&p);
        simular.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.simulate();
            }
        });
        p
    }

    /// The page for this manifest (or what is wrong with it).
    pub fn open(self: &Rc<Self>, path: &Path) {
        clear(&self.corpo);
        clear(&self.lado);
        clear(&self.resultado);
        self.instalando.set(false);
        match read(path) {
            Ok(m) => {
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug instalar agente: «{}» de {} ({} permissões pedidas{})", m.nome, if m.editor.is_empty() { "?" } else { &m.editor }, m.permissoes.len(),
                        if crate::app_store_data::in_catalog(path) { ", do catálogo" } else { "" });
                }
                *self.ligadas.borrow_mut() = vec![true; m.permissoes.len()];
                *self.autonomia.borrow_mut() = m.autonomia.clone();
                *self.manifesto.borrow_mut() = Some((path.to_path_buf(), m));
                self.render();
            }
            Err(e) => {
                *self.manifesto.borrow_mut() = None;
                let t = rotulo(tr("This agent cannot be installed"), "central-titulo");
                t.set_xalign(0.0);
                self.corpo.append(&t);
                let l = rotulo(&e, "central-agente-sub");
                l.set_xalign(0.0);
                l.set_wrap(true);
                self.corpo.append(&l);
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug instalar agente: recusado ({e})");
                }
            }
        }
    }

    fn render(self: &Rc<Self>) {
        let Some((_, m)) = self.manifesto.borrow().clone() else { return };
        clear(&self.corpo);
        // the header: back, the agent, where it comes from, the main actions
        let voltar = gtk::Button::new();
        voltar.add_css_class("cfg-link");
        let vb = caixa(Orientation::Horizontal, 4, "");
        vb.append(&ui_icon("anterior", "#8E8F99", 12));
        vb.append(&rotulo(tr("Agents"), ""));
        voltar.set_child(Some(&vb));
        voltar.set_halign(Align::Start);
        let w = Rc::downgrade(self);
        voltar.connect_clicked(move |_| {
            if let Some(f) = w.upgrade().as_ref().and_then(|p| p.ir.borrow().as_ref().map(|_| ())).and(w.upgrade()) {
                if let Some(f) = f.ir.borrow().as_ref() {
                    f("agentes");
                }
            }
        });
        self.corpo.append(&voltar);
        let cab = caixa(Orientation::Horizontal, 16, "");
        cab.set_margin_top(12);
        let ic = caixa(Orientation::Horizontal, 0, "instalar-agente-icone");
        ic.set_size_request(64, 64);
        ic.set_valign(Align::Start);
        let st = estrela(28, "#8ED2C3");
        st.set_hexpand(true);
        st.set_halign(Align::Center);
        ic.append(&st);
        cab.append(&ic);
        let col = caixa(Orientation::Vertical, 4, "");
        col.set_hexpand(true);
        let n = rotulo(&m.nome, "central-titulo");
        n.set_xalign(0.0);
        col.append(&n);
        let origem = caixa(Orientation::Horizontal, 6, "");
        origem.append(&rotulo(if m.editor.is_empty() { tr("unknown publisher") } else { &m.editor }, "central-agente-sub"));
        origem.append(&rotulo("·", "central-agente-sub"));
        // (a file from anywhere: nothing here vouches for it — said, not hidden; one of the system's catalog is the
        // App Center's, shipped with KalangoOS)
        if self.manifesto.borrow().as_ref().is_some_and(|(p, _)| crate::app_store_data::in_catalog(p)) {
            origem.append(&ui_icon("check", "#8ED2C3", 12));
            origem.append(&rotulo(tr("App Center · KalangoOS catalog"), "central-agente-sub"));
        } else {
            origem.append(&ui_icon("alerta", "#E5C98C", 12));
            origem.append(&rotulo(tr("local file · not verified"), "instalar-agente-aviso"));
        }
        col.append(&origem);
        cab.append(&col);
        let botoes = caixa(Orientation::Vertical, 8, "");
        botoes.set_valign(Align::Start);
        let ib = caixa(Orientation::Horizontal, 6, "");
        ib.set_halign(Align::Center);
        ib.append(&ui_icon("baixar", "#1C1D24", 13));
        ib.append(&rotulo(tr("Install with these permissions"), ""));
        self.instalar.set_child(Some(&ib));
        self.instalar.set_sensitive(true);
        let sb = caixa(Orientation::Horizontal, 6, "");
        sb.set_halign(Align::Center);
        sb.append(&ui_icon("tocar", "#C9CAD1", 12));
        sb.append(&rotulo(tr("Try it in simulation"), ""));
        self.simular.set_child(Some(&sb));
        for b in [&self.instalar, &self.simular] {
            if let Some(par) = b.parent() {
                if let Ok(bx) = par.downcast::<gtk::Box>() {
                    bx.remove(b);
                }
            }
            botoes.append(b);
        }
        cab.append(&botoes);
        self.corpo.append(&cab);
        if !m.descricao.is_empty() {
            let d = rotulo(&m.descricao, "desk-metrica-sub");
            d.set_xalign(0.0);
            d.set_wrap(true);
            d.set_margin_top(14);
            self.corpo.append(&d);
        }
        // what it may do — each one can be switched off before installing
        self.corpo.append(&secao(tr("WHAT IT WILL BE ABLE TO DO — ADJUST BEFORE INSTALLING"), "escudo"));
        let lista = caixa(Orientation::Vertical, 0, "central-cartao");
        for (i, p) in m.permissoes.iter().enumerate() {
            let (icone, titulo) = match p.tipo.as_str() {
                "ler" => ("olho", trf("Read {folders}", &[("folders", p.pastas.join(", "))])),
                "escrever" => ("pasta", trf("Change files in {folders}", &[("folders", p.pastas.join(", "))])),
                "apagar" => ("lixeira", trf("Delete files in {folders}", &[("folders", p.pastas.join(", "))])),
                "programas" => ("terminal", trf("Run {programs}", &[("programs", p.programas.join(", "))])),
                "rede" => ("rede-cabo", if p.dominios.is_empty() { tr("Network: any site").to_string() } else { trf("Network: only {sites}", &[("sites", p.dominios.join(", "))]) }),
                _ => ("monitor", tr("See your screen").to_string()),
            };
            let sub = if !p.detalhe.is_empty() {
                p.detalhe.clone()
            } else if p.tipo == "rede" && !p.dominios.is_empty() {
                tr("any other site is blocked").to_string()
            } else {
                String::new()
            };
            let sw = gtk::Switch::new();
            sw.add_css_class("central-switch");
            sw.set_active(self.ligadas.borrow().get(i).copied().unwrap_or(true));
            let w = Rc::downgrade(self);
            sw.connect_state_set(move |_, v| {
                if let Some(p) = w.upgrade() {
                    if let Some(x) = p.ligadas.borrow_mut().get_mut(i) {
                        *x = v;
                    }
                    p.render_trust();
                    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                        eprintln!("debug instalar agente: permissão {i} {v}");
                    }
                }
                glib::Propagation::Proceed
            });
            crate::central::posicao_debug(&format!("instalar agente permissao {i}"), sw.upcast_ref());
            lista.append(&linha(icone, &titulo, &sub, &sw));
        }
        // what it never gets: the rest of the folders, when it asked for some
        let pastas: Vec<String> = m.permissoes.iter().flat_map(|p| p.pastas.clone()).collect();
        if !pastas.is_empty() {
            let nunca = rotulo(tr("never"), "novo-agente-tag");
            nunca.add_css_class("cinza");
            lista.append(&linha("cadeado", tr("Read other folders"), tr("Documents, Pictures, ~/.ssh and the rest"), &nunca));
        }
        if m.permissoes.is_empty() {
            let v = rotulo(tr("It asks for no permission: it can only answer."), "central-agente-sub");
            v.set_xalign(0.0);
            v.set_margin_top(8);
            v.set_margin_bottom(8);
            lista.append(&v);
        }
        self.corpo.append(&lista);
        // autonomy, with what it means
        self.corpo.append(&secao(tr("AUTONOMY"), "escudo"));
        let linha_a = caixa(Orientation::Horizontal, 12, "");
        let seg = caixa(Orientation::Horizontal, 2, "cfg-segmentado");
        seg.add_css_class("escuro");
        let explica = rotulo("", "central-agente-sub");
        explica.set_xalign(0.0);
        explica.set_wrap(true);
        explica.set_hexpand(true);
        let mut primeiro: Option<gtk::ToggleButton> = None;
        for (id, nome) in [("sempre", tr("Always ask")), ("risco", tr("Only risky actions")), ("autonomo", tr("Autonomous"))] {
            let b = gtk::ToggleButton::with_label(nome);
            b.add_css_class("cfg-seg");
            if let Some(f) = &primeiro {
                b.set_group(Some(f));
            } else {
                primeiro = Some(b.clone());
            }
            b.set_active(*self.autonomia.borrow() == id);
            let (w, e) = (Rc::downgrade(self), explica.clone());
            b.connect_toggled(move |b| {
                if let Some(p) = w.upgrade().filter(|_| b.is_active()) {
                    *p.autonomia.borrow_mut() = id.to_string();
                    e.set_text(explain(id));
                    p.render_trust();
                }
            });
            seg.append(&b);
        }
        explica.set_text(explain(&self.autonomia.borrow()));
        linha_a.append(&seg);
        linha_a.append(&explica);
        self.corpo.append(&linha_a);
        self.corpo.append(&self.resultado);
        self.render_trust();
    }

    /// The trust summary: what is true of it with the permissions as they are now (redrawn as they change).
    fn render_trust(&self) {
        let Some((caminho, m)) = self.manifesto.borrow().clone() else { return };
        clear(&self.lado);
        // (one of the App Center's catalog comes with the system; a file from anywhere is not vouched for)
        let catalogo = crate::app_store_data::in_catalog(&caminho);
        let (pastas, _programas, dominios, desligadas) = scope(&m, &self.ligadas.borrow());
        self.lado.append(&secao(tr("TRUST SUMMARY"), "escudo"));
        let rede = !desligadas.contains(&"rede".to_string());
        let itens: Vec<(&str, String, String)> = vec![
            if catalogo {
                ("check", tr("From KalangoOS's catalog").to_string(), tr("it comes with the system, through the App Center").to_string())
            } else {
                ("alerta", tr("Not verified").to_string(), tr("a file from outside the system: check who made it").to_string())
            },
            ("cpu", tr("Runs on the local model").to_string(), if rede { tr("it reaches only the sites below").to_string() } else { tr("nothing leaves the computer").to_string() }),
            ("olho", if desligadas.contains(&"tela".to_string()) { tr("It does not see your screen") } else { tr("It sees your screen") }.to_string(), tr("nor keyboard, microphone or camera").to_string()),
            ("rede-cabo", if !rede { tr("No network").to_string() } else if dominios.is_empty() { tr("Network open").to_string() } else { tr("Restricted network").to_string() },
                if !rede { String::new() } else if dominios.is_empty() { tr("any site").to_string() } else if dominios.len() == 1 { tr("1 site allowed").to_string() } else { trf("{n} sites allowed", &[("n", dominios.len().to_string())]) }),
            ("pasta", if pastas.is_empty() { tr("No folder").to_string() } else { pastas.join(", ") }, tr("the folders it may use").to_string()),
            ("relogio", tr("Everything can be undone").to_string(), tr("each action goes into the Timeline").to_string()),
        ];
        let card = caixa(Orientation::Vertical, 0, "central-cartao");
        for (icone, t, sub) in itens {
            let r = caixa(Orientation::Horizontal, 10, "novo-agente-permissao");
            r.append(&ui_icon(icone, match icone { "alerta" => "#E5C98C", "check" => "#8ED2C3", _ => "#8E8F99" }, 15));
            let c = caixa(Orientation::Vertical, 1, "");
            c.set_hexpand(true);
            let tl = rotulo(&t, "central-limite-nome");
            tl.set_xalign(0.0);
            tl.set_ellipsize(gtk::pango::EllipsizeMode::End);
            tl.set_max_width_chars(1);
            c.append(&tl);
            if !sub.is_empty() {
                let s = rotulo(&sub, "central-agente-sub");
                s.set_xalign(0.0);
                s.set_wrap(true);
                c.append(&s);
            }
            r.append(&c);
            card.append(&r);
        }
        self.lado.append(&card);
        let sim = caixa(Orientation::Vertical, 4, "novo-agente-simulacao");
        sim.set_margin_top(14);
        let st = caixa(Orientation::Horizontal, 6, "");
        st.append(&ui_icon("tocar", "#C9CAD1", 12));
        st.append(&rotulo(tr("Simulation mode"), "central-limite-nome"));
        sim.append(&st);
        let sd = rotulo(tr("The agent runs for real, but instead of doing it shows what it would do. Good for getting to know it before giving it permissions."), "central-agente-sub");
        sd.set_xalign(0.0);
        sd.set_wrap(true);
        sim.append(&sd);
        self.lado.append(&sim);
        let e = caixa(Orientation::Vertical, 0, "");
        e.set_vexpand(true);
        self.lado.append(&e);
        let n = rotulo(tr("You can remove it later in Agents › Routines."), "central-agente-sub");
        n.set_xalign(0.0);
        n.set_wrap(true);
        self.lado.append(&n);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug instalar agente: confiança · pastas {} · sites {} · sem {}", pastas.join(","), dominios.join(","), desligadas.join(","));
        }
    }

    /// The agent's arguments for this scope (the same for a simulation and for the installed routine).
    fn config(&self) -> Option<(Manifesto, AgenteConfig, Vec<String>)> {
        let (_, m) = self.manifesto.borrow().clone()?;
        let (pastas, programas, dominios, desligadas) = scope(&m, &self.ligadas.borrow());
        let mut extra = Vec::new();
        if !pastas.is_empty() {
            extra.push(format!("--pastas={}", pastas.join(",")));
        }
        if !programas.is_empty() {
            extra.push(format!("--programas={}", programas.join(",")));
        }
        if !dominios.is_empty() {
            extra.push(format!("--dominios={}", dominios.join(",")));
        }
        let cfg = AgenteConfig {
            descricao: m.instrucoes.clone(),
            passos: Vec::new(),
            autonomia: self.autonomia.borrow().clone(),
            desligadas,
            modelo: String::new(),
            pastas,
            programas,
            dominios,
            origem: m.id.clone(),
        };
        Some((m, cfg, extra))
    }

    /// Runs it once with nothing changed: what it would do, step by step, and its answer.
    fn simulate(self: &Rc<Self>) {
        let Some((m, cfg, extra)) = self.config() else { return };
        clear(&self.resultado);
        self.instalar_fim.replace(None);
        self.resultado.append(&secao(tr("SIMULATION"), "tocar"));
        let estado = rotulo(tr("The agent is running in simulation…"), "central-agente-sub");
        estado.set_xalign(0.0);
        self.resultado.append(&estado);
        self.simular.set_sensitive(false);
        let pedido = cfg.pedido(&m.nome);
        let mut args: Vec<String> = vec!["--json".into(), "--simular".into(), format!("--desligadas={}", cfg.desligadas.join(",")), format!("--autonomia={}", cfg.autonomia)];
        args.extend(extra);
        args.push(pedido);
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || -> Result<(Vec<String>, String), String> {
            let o = std::process::Command::new(crate::voz::agent_binary()).args(&args).stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null()).output().map_err(|e| e.to_string())?;
            let (mut passos, mut resposta) = (Vec::new(), String::new());
            // one line per step: what it was (the tool's words), whether it went, and in a few words what came of it
            let mut descricao = String::new();
            for l in String::from_utf8_lossy(&o.stdout).lines() {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else { continue };
                match v["tipo"].as_str() {
                    Some("ferramenta") => descricao = v["descricao"].as_str().unwrap_or("").split(" [").next().unwrap_or("").trim_end_matches('.').to_string(),
                    Some("resultado") => {
                        let ok = v["ok"].as_bool().unwrap_or(false);
                        let resumo = v["resumo"].as_str().filter(|r| !r.is_empty()).map(str::to_string)
                            .unwrap_or_else(|| v["texto"].as_str().unwrap_or("").lines().next().unwrap_or("").chars().take(90).collect());
                        let marca = if resumo.contains(tr("simulated: nothing was changed")) { "◇" } else if ok { "✓" } else { "✗" };
                        let resumo = if resumo.trim_start().starts_with('{') { String::new() } else { resumo };
                        passos.push(if resumo.is_empty() { format!("{marca} {descricao}") } else { format!("{marca} {descricao} — {resumo}") });
                    }
                    Some("resposta") => resposta = v["texto"].as_str().unwrap_or("").to_string(),
                    Some("erro") => return Err(v["texto"].as_str().unwrap_or("").to_string()),
                    _ => {}
                }
            }
            Ok((passos, resposta))
        }, move |r| {
            let Some(p) = w.upgrade() else { return };
            p.simular.set_sensitive(true);
            match r {
                Ok((passos, resposta)) => {
                    estado.set_text(&trf("{n} steps — nothing was changed", &[("n", passos.len().to_string())]));
                    let caixa_sim = caixa(Orientation::Vertical, 6, "novo-agente-simulacao");
                    for t in &passos {
                        let l = rotulo(t, "novo-agente-sim");
                        l.set_xalign(0.0);
                        l.set_wrap(true);
                        caixa_sim.append(&l);
                    }
                    let legenda = rotulo(tr("✓ done (reading only) · ◇ simulated: not done · ✗ refused"), "central-agente-sub");
                    legenda.set_xalign(0.0);
                    caixa_sim.append(&legenda);
                    if !resposta.is_empty() {
                        caixa_sim.append(&crate::markdown::render(&resposta));
                    }
                    // the decision again, where the person is now (the page scrolled to the result: the button at the
                    // top is out of sight)
                    let fim = gtk::Button::new();
                    fim.add_css_class("botao-principal");
                    fim.set_halign(Align::Start);
                    fim.set_margin_top(8);
                    let fb = caixa(Orientation::Horizontal, 6, "");
                    fb.append(&ui_icon("baixar", "#1C1D24", 13));
                    fb.append(&rotulo(tr("Install with these permissions"), ""));
                    fim.set_child(Some(&fb));
                    let w = Rc::downgrade(&p);
                    fim.connect_clicked(move |_| {
                        if let Some(p) = w.upgrade() {
                            p.install();
                        }
                    });
                    crate::central::posicao_debug_sempre("instalar agente instalar fim", fim.upcast_ref());
                    caixa_sim.append(&fim);
                    p.instalar_fim.replace(Some(fim));
                    p.resultado.append(&caixa_sim);
                    // (the result is below the fold: the page scrolls to it)
                    if let Some(sw) = p.resultado.ancestor(gtk::ScrolledWindow::static_type()).and_downcast::<gtk::ScrolledWindow>() {
                        glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
                            let a = sw.vadjustment();
                            a.set_value(a.upper() - a.page_size());
                        });
                    }
                    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                        eprintln!("debug instalar agente: simulação com {} passos ({} simulados)", passos.len(), passos.iter().filter(|t| t.starts_with('◇')).count());
                    }
                }
                Err(e) => estado.set_text(&trf("The simulation did not run: {error}", &[("error", e)])),
            }
        });
    }

    /// Installed: a routine with this scope; the button becomes a progress bar (the slow spring), then the routines.
    fn install(self: &Rc<Self>) {
        if self.instalando.replace(true) {
            return;
        }
        let Some((m, cfg, _)) = self.config() else { return };
        let gatilho = match &m.quando {
            Some(q) => Gatilho::Horario { dias: q.dias.clone(), hora: q.hora, minuto: q.minuto },
            None => Gatilho::Manual,
        };
        let pedido = cfg.pedido(&m.nome);
        let mut r = Rotinas::carregar();
        let id = r.adicionar(&m.nome, Acao::Pedido(pedido), gatilho);
        if let Some(x) = r.lista.iter_mut().find(|x| x.id == id) {
            x.agente = Some(cfg.clone());
        }
        r.salvar();
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug instalar agente: instalado «{}» (pastas {} · programas {} · sites {} · sem {})", m.nome, cfg.pastas.join(","), cfg.programas.join(","), cfg.dominios.join(","), cfg.desligadas.join(","));
        }
        let barra = gtk::LevelBar::for_interval(0.0, 1.0);
        barra.add_css_class("plano-barra");
        barra.set_size_request(200, -1);
        barra.set_valign(Align::Center);
        self.instalar.set_child(Some(&barra));
        self.instalar.set_sensitive(false);
        // (the one under the simulation shows the same progress)
        if let Some(f) = self.instalar_fim.borrow().as_ref() {
            let b2 = gtk::LevelBar::for_interval(0.0, 1.0);
            b2.add_css_class("plano-barra");
            b2.set_size_request(200, -1);
            b2.set_valign(Align::Center);
            f.set_child(Some(&b2));
            f.set_sensitive(false);
            let fw = f.downgrade();
            crate::disk_answer::grow(&b2, move |x| {
                if x >= 0.999 {
                    if let Some(f) = fw.upgrade() {
                        f.set_child(Some(&rotulo(tr("Already installed"), "")));
                    }
                } else if let Some(b) = f_bar(&fw) {
                    b.set_value(x);
                }
            });
        }
        let bw = barra.downgrade();
        let w = Rc::downgrade(self);
        crate::disk_answer::grow(&barra, move |x| {
            if let Some(b) = bw.upgrade() {
                b.set_value(x);
                if x >= 0.999 {
                    if let Some(p) = w.upgrade() {
                        p.instalar.set_child(Some(&rotulo(tr("Already installed"), "")));
                        let w2 = Rc::downgrade(&p);
                        glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
                            if let Some(p) = w2.upgrade() {
                                if let Some(f) = p.ir.borrow().as_ref() {
                                    f("rotinas");
                                }
                            }
                        });
                    }
                }
            }
        });
    }
}

fn explain(id: &str) -> &'static str {
    match id {
        "sempre" => tr("asks before every action that changes something"),
        "autonomo" => tr("does not ask: it acts within these limits"),
        _ => tr("asks before deleting, running programs or using the network"),
    }
}

fn linha(icone: &str, titulo: &str, sub: &str, controle: &impl IsA<gtk::Widget>) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 10, "novo-agente-permissao");
    r.append(&ui_icon(icone, "#8E8F99", 15));
    let c = caixa(Orientation::Vertical, 1, "");
    c.set_hexpand(true);
    c.set_valign(Align::Center);
    let t = rotulo(titulo, "central-limite-nome");
    t.set_xalign(0.0);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    t.set_max_width_chars(1);
    c.append(&t);
    if !sub.is_empty() {
        let s = rotulo(sub, "central-agente-sub");
        s.set_xalign(0.0);
        s.set_ellipsize(gtk::pango::EllipsizeMode::End);
        s.set_max_width_chars(1);
        c.append(&s);
    }
    r.append(&c);
    controle.set_valign(Align::Center);
    r.append(controle);
    r
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEV_SYNC: &str = r#"
nome = "Dev Sync"
id = "labs.devsync"
editor = "KalangoOS Labs"
descricao = "Mantém seus repositórios sincronizados."
instrucoes = "Para cada repositório git em ~/Projetos, rode git fetch e diga quais têm mudanças."
autonomia = "risco"
[quando]
dias = [1, 2, 3, 4, 5, 9]
hora = 30
[[permissoes]]
tipo = "ler"
pastas = ["~/Projetos"]
detalhe = "somente as pastas com repositório git"
[[permissoes]]
tipo = "programas"
programas = ["git"]
[[permissoes]]
tipo = "rede"
dominios = ["github.com"]
[[permissoes]]
tipo = "formatar-disco"
"#;

    fn manifesto() -> Manifesto {
        // (one file per call: the tests run at the same time)
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("kalango-dev-sync-{}-{n}.kalango-agente", std::process::id()));
        std::fs::write(&p, DEV_SYNC).unwrap();
        let m = read(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        m
    }

    #[test]
    fn a_manifest_is_read_and_checked() {
        let m = manifesto();
        assert_eq!(m.nome, "Dev Sync");
        assert_eq!(m.permissoes.len(), 3, "an unknown kind of permission is dropped");
        assert_eq!(m.quando, Some(Quando { dias: vec![1, 2, 3, 4, 5], hora: 23, minuto: 0 }));
    }

    #[test]
    fn the_scope_is_what_was_asked_and_left_on() {
        let m = manifesto();
        let (pastas, programas, dominios, off) = scope(&m, &[true, true, true]);
        assert_eq!((pastas, programas, dominios), (vec!["~/Projetos".to_string()], vec!["git".to_string()], vec!["github.com".to_string()]));
        assert_eq!(off, vec!["escrever", "apagar", "tela", "gastar"]);
        // the network switched off before installing
        let (_, _, dominios, off) = scope(&m, &[true, true, false]);
        assert!(dominios.is_empty() && off.contains(&"rede".to_string()));
    }

    #[test]
    fn a_broken_manifest_is_refused() {
        let p = std::env::temp_dir().join(format!("kalango-quebrado-{}.kalango-agente", std::process::id()));
        std::fs::write(&p, "nome = \"\"\ninstrucoes = \"x\"\n").unwrap();
        assert!(read(&p).is_err());
        std::fs::write(&p, "isto não é toml = = =").unwrap();
        assert!(read(&p).is_err());
        let _ = std::fs::remove_file(&p);
    }
}
