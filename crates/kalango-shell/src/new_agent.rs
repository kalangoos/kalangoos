//! "Criar agente (por descrição)" (design "Criar agente (por descrição)"): the person writes a sentence; the local model
//! returns a name, when it runs, its steps and how much it asks (`kalango-agente agente-de-descricao`). The page shows
//! the minimal permissions those steps need and a simulation of a run now — counted here, in the folders the steps
//! name, nothing changed. "Criar agente" keeps it as a routine that runs with that scope (the columns it does not need
//! turned off, its autonomy); "Salvar rascunho" keeps it paused. Each block comes in as it is made (the 24 ms
//! cascade); a new sentence redraws only what changed.

use crate::conversa::ui_icon;
use crate::rotinas::{Acao, AgenteConfig, Gatilho, Rotinas};
use crate::superficies::{caixa, estrela, rotulo};

use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use kalango_movimento::{self as mv, Animavel};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// One step of the agent: what it does, its kind of action, the folder, the file kinds and their age.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Step {
    pub texto: String,
    pub acao: String,
    pub pasta: String,
    pub tipos: Vec<String>,
    pub idade_dias: u64,
}

/// What the model understood: a name, when it runs (days 1–7, hour, minute; None: when the person asks), the steps
/// and how much it asks ("sempre", "risco", "autonomo").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spec {
    pub nome: String,
    pub quando: Option<(Vec<u8>, u8, u8)>,
    pub passos: Vec<Step>,
    pub autonomia: String,
}

impl Spec {
    pub fn from_json(v: &Value) -> Spec {
        let q = &v["quando"];
        let quando = (q["tipo"] == "horario").then(|| {
            let dias = q["dias"].as_array().into_iter().flatten().filter_map(|d| d.as_u64()).filter(|d| (1..=7).contains(d)).map(|d| d as u8).collect();
            (dias, q["hora"].as_u64().unwrap_or(18).min(23) as u8, q["minuto"].as_u64().unwrap_or(0).min(59) as u8)
        });
        let passos = v["passos"].as_array().into_iter().flatten().map(|p| Step {
            texto: p["texto"].as_str().unwrap_or("").to_string(),
            acao: p["acao"].as_str().unwrap_or("ler").to_string(),
            pasta: p["pasta"].as_str().unwrap_or("").to_string(),
            tipos: p["tipos"].as_array().into_iter().flatten().filter_map(|t| t.as_str().map(str::to_string)).collect(),
            idade_dias: p["idade_dias"].as_u64().unwrap_or(0),
        }).filter(|p| !p.texto.is_empty()).collect();
        Spec { nome: v["nome"].as_str().unwrap_or("").to_string(), quando, passos, autonomia: v["autonomia"].as_str().unwrap_or("risco").to_string() }
    }

    fn gatilho(&self) -> Gatilho {
        match &self.quando {
            Some((dias, hora, minuto)) => Gatilho::Horario { dias: dias.clone(), hora: *hora, minuto: *minuto },
            None => Gatilho::Manual,
        }
    }
}

/// Whether a step of this kind asks the person first under an autonomy.
pub fn asks(acao: &str, autonomia: &str) -> bool {
    match autonomia {
        "autonomo" => false,
        "sempre" => !matches!(acao, "ler" | "notificar"),
        _ => matches!(acao, "apagar" | "rede" | "terminal"),
    }
}

/// The permission columns the agent runs without: what its steps do not need (and the screen and spending, which
/// no step of this kind uses); reading and changing files also when the person switched them off.
pub fn columns_off(passos: &[Step], ler: bool, mudar: bool) -> Vec<String> {
    let precisa = |a: &[&str]| passos.iter().any(|p| a.contains(&p.acao.as_str()));
    let mut off: Vec<&str> = Vec::new();
    if !ler {
        off.push("ler");
    }
    if !mudar || !precisa(&["mover", "escrever"]) {
        off.push("escrever");
    }
    for (col, acoes) in [("apagar", &["apagar"][..]), ("rede", &["rede"][..]), ("terminal", &["terminal"][..])] {
        if !precisa(acoes) {
            off.push(col);
        }
    }
    off.extend(["tela", "gastar"]);
    off.into_iter().map(str::to_string).collect()
}

/// A file's kind, for "moveria N arquivos para K pastas".
fn kind(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "pdf" | "doc" | "docx" | "odt" | "txt" | "md" | "rtf" | "xls" | "xlsx" | "ods" | "csv" | "ppt" | "pptx" | "odp" | "epub" => "documentos",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "heic" | "bmp" | "tif" | "tiff" => "imagens",
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" => "videos",
        "mp3" | "flac" | "ogg" | "wav" | "m4a" | "opus" | "aac" => "musicas",
        "zip" | "tar" | "gz" | "xz" | "7z" | "rar" | "zst" | "bz2" | "tgz" => "compactados",
        "deb" | "rpm" | "appimage" | "flatpakref" | "run" | "exe" | "msi" | "dmg" => "instaladores",
        _ => "outros",
    }
}

/// The files directly in a folder (not hidden) of the given kinds (any when none) older than `idade_dias` (any when
/// 0): (path, bytes).
fn files(dir: &Path, tipos: &[String], idade_dias: u64) -> Vec<(PathBuf, u64)> {
    let agora = std::time::SystemTime::now();
    std::fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|e| {
        let p = e.path();
        let nome = p.file_name()?.to_string_lossy().into_owned();
        let m = e.metadata().ok().filter(|m| m.is_file())?;
        if nome.starts_with('.') {
            return None;
        }
        let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !tipos.is_empty() && !tipos.iter().any(|t| t.eq_ignore_ascii_case(&ext)) {
            return None;
        }
        if idade_dias > 0 {
            let idade = m.modified().ok().and_then(|t| agora.duration_since(t).ok()).map(|d| d.as_secs() / 86_400).unwrap_or(0);
            if idade < idade_dias {
                return None;
            }
        }
        Some((p, m.len()))
    }).collect()
}

/// A run now, counted (nothing is changed): one line per step — (it would ask first, what it would do).
pub fn simulate(passos: &[Step], autonomia: &str) -> Vec<(bool, String)> {
    passos.iter().map(|p| {
        let pede = asks(&p.acao, autonomia);
        let pasta = (!p.pasta.trim().is_empty()).then(|| crate::pastas::resolver(&p.pasta));
        let nome_pasta = if p.pasta.trim().is_empty() { tr("the folder").to_string() } else { p.pasta.trim().to_string() };
        let lista = || pasta.as_ref().map(|d| files(d, &p.tipos, p.idade_dias));
        let texto = match p.acao.as_str() {
            "mover" => match lista() {
                Some(f) if !f.is_empty() => {
                    let mut tipos: Vec<&str> = f.iter().map(|(x, _)| kind(&x.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default())).collect();
                    tipos.sort_unstable();
                    tipos.dedup();
                    trf("would move {n} files into {k} folders", &[("n", f.len().to_string()), ("k", tipos.len().to_string())])
                }
                Some(_) => trf("would move nothing now: {folder} has no such file", &[("folder", nome_pasta)]),
                None => tr("would move files (no folder was named)").to_string(),
            },
            "apagar" => match lista() {
                Some(f) if !f.is_empty() => {
                    let bytes: u64 = f.iter().map(|x| x.1).sum();
                    let size = glib::format_size(bytes).to_string();
                    if pede {
                        trf("would ask to delete {n} files ({size})", &[("n", f.len().to_string()), ("size", size)])
                    } else {
                        trf("would delete {n} files ({size}): to the trash for 30 days", &[("n", f.len().to_string()), ("size", size)])
                    }
                }
                Some(_) => trf("would delete nothing now: no file in {folder} matches", &[("folder", nome_pasta)]),
                None => tr("would delete files (no folder was named)").to_string(),
            },
            "ler" => match lista() {
                Some(f) => trf("would read {n} files in {folder}", &[("n", f.len().to_string()), ("folder", nome_pasta)]),
                None => tr("would read what the step names").to_string(),
            },
            "escrever" => trf("would write in {folder}", &[("folder", nome_pasta)]),
            "notificar" => tr("would show a summary on the island").to_string(),
            "rede" => if pede { tr("would ask to use the network") } else { tr("would use the network") }.to_string(),
            "terminal" => if pede { tr("would ask to run a program") } else { tr("would run a program") }.to_string(),
            _ => p.texto.clone(),
        };
        (pede, texto)
    }).collect()
}

/// The kind of a step the person adds by hand, from its words.
fn guess_action(t: &str) -> &'static str {
    let t = t.to_lowercase();
    let tem = |ws: &[&str]| ws.iter().any(|w| t.contains(w));
    if tem(&["apag", "exclu", "remov", "delete"]) {
        "apagar"
    } else if tem(&["mov", "separ", "organiz", "arrum", "move"]) {
        "mover"
    } else if tem(&["avis", "notific", "mostr", "mand", "resum", "notify", "show"]) {
        "notificar"
    } else if tem(&["baix", "site", "web", "internet", "download"]) {
        "rede"
    } else if tem(&["rode", "rodar", "execut", "comando", "run "]) {
        "terminal"
    } else if tem(&["escrev", "cri", "salv", "write", "save"]) {
        "escrever"
    } else {
        "ler"
    }
}

const TEMPLATES: &[&str] = &[
    "Toda sexta, organize meus downloads, apague instaladores com mais de 30 dias e me mande um resumo do que fez.",
    "Todo dia às 9h, leia os arquivos novos em Documentos e me mostre um resumo.",
    "Toda segunda às 8h, apague arquivos compactados com mais de 60 dias em Downloads.",
];

pub struct NewAgentPage {
    pub raiz: gtk::Box,
    texto: gtk::TextView,
    gerar: gtk::Button,
    estado: gtk::Label,
    entendeu: gtk::Box,
    nome: gtk::Entry,
    quando: gtk::MenuButton,
    quando_rotulo: gtk::Label,
    modelo: gtk::DropDown,
    modelos: Vec<PathBuf>,
    autonomia: Vec<(&'static str, gtk::ToggleButton)>,
    passos_box: gtk::Box,
    passos_secao: gtk::Box,
    permissoes_box: gtk::Box,
    simulacao_box: gtk::Box,
    lado: gtk::Box,
    criar: gtk::Button,
    rascunho: gtk::Button,
    spec: RefCell<Option<Spec>>,
    /// what was drawn last (a new sentence animates only the rows that changed)
    desenhado: RefCell<(Vec<String>, Vec<String>, Vec<String>)>,
    ler: Cell<bool>,
    mudar: Cell<bool>,
    geracao: Cell<u64>,
    preenchendo: Cell<bool>,
    /// after "Criar agente": where to go (the routines page)
    pub ir: RefCell<Option<Box<dyn Fn(&str)>>>,
}

fn secao(t: &str, icone: &str) -> gtk::Box {
    let l = caixa(Orientation::Horizontal, 8, "");
    l.set_margin_top(18);
    l.set_margin_bottom(10);
    if icone == "spark" { l.append(&estrela(12, "#A5A1E2")) } else { l.append(&ui_icon(icone, "#8E8F99", 13)) }
    l.append(&rotulo(t, "central-painel-secao"));
    l
}

/// A row of the "understood" card: icon, label, and its control at the right.
fn campo(icone: &str, nome: &str, controle: &impl IsA<gtk::Widget>) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 10, "novo-agente-campo");
    r.append(&ui_icon(icone, "#8E8F99", 15));
    let l = rotulo(nome, "central-limite-nome");
    l.set_xalign(0.0);
    l.set_hexpand(true);
    r.append(&l);
    controle.set_valign(Align::Center);
    r.append(controle);
    r
}

impl NewAgentPage {
    pub fn new() -> Rc<NewAgentPage> {
        let raiz = caixa(Orientation::Horizontal, 0, "novo-agente");
        raiz.set_vexpand(true);
        // ---- left: the sentence and what was understood
        let esq = caixa(Orientation::Vertical, 0, "novo-agente-esq");
        esq.set_hexpand(true);
        let t = rotulo(tr("New agent"), "central-titulo");
        t.set_xalign(0.0);
        esq.append(&t);
        let st = rotulo(tr("Describe what it should do. The system puts the agent together and shows everything before creating it."), "desk-metrica-sub");
        st.set_xalign(0.0);
        st.set_wrap(true);
        esq.append(&st);
        let comp = caixa(Orientation::Vertical, 10, "conversas-compositor");
        comp.add_css_class("novo-agente-frase");
        comp.set_margin_top(16);
        let texto = gtk::TextView::new();
        texto.set_wrap_mode(gtk::WrapMode::WordChar);
        texto.add_css_class("conversas-texto");
        texto.set_accepts_tab(false);
        let dica = rotulo(tr("Every Friday, tidy my downloads…"), "conversas-placeholder");
        let ov = gtk::Overlay::new();
        ov.set_child(Some(&texto));
        dica.set_halign(Align::Start);
        dica.set_valign(Align::Start);
        dica.set_can_target(false);
        ov.add_overlay(&dica);
        comp.append(&ov);
        let barra = caixa(Orientation::Horizontal, 8, "");
        let exemplo = gtk::Button::new();
        exemplo.add_css_class("conversas-ferramenta");
        let eb = caixa(Orientation::Horizontal, 6, "");
        eb.append(&ui_icon("clipe", "#C9CAD1", 13));
        eb.append(&rotulo(tr("attach an example"), ""));
        exemplo.set_child(Some(&eb));
        barra.append(&exemplo);
        let modelo_b = gtk::MenuButton::new();
        modelo_b.add_css_class("conversas-ferramenta");
        let mb = caixa(Orientation::Horizontal, 6, "");
        mb.append(&ui_icon("camadas", "#C9CAD1", 13));
        mb.append(&rotulo(tr("from a template"), ""));
        modelo_b.set_child(Some(&mb));
        modelo_b.set_always_show_arrow(false);
        barra.append(&modelo_b);
        let esp = caixa(Orientation::Horizontal, 0, "");
        esp.set_hexpand(true);
        barra.append(&esp);
        let gerar = gtk::Button::new();
        gerar.add_css_class("conversas-enviar");
        gerar.set_child(Some(&ui_icon("enviar", "#1C1D24", 15)));
        gerar.set_tooltip_text(Some(tr("Put the agent together")));
        barra.append(&gerar);
        comp.append(&barra);
        esq.append(&comp);
        let estado = rotulo("", "central-agente-sub");
        estado.set_xalign(0.0);
        estado.set_margin_top(8);
        estado.set_visible(false);
        esq.append(&estado);

        let entendeu = caixa(Orientation::Vertical, 0, "");
        entendeu.append(&secao(tr("THE AGENT UNDERSTOOD IT LIKE THIS"), "spark"));
        let card = caixa(Orientation::Vertical, 0, "central-cartao");
        let nome = gtk::Entry::new();
        nome.add_css_class("novo-agente-nome");
        nome.set_width_chars(18);
        card.append(&campo("usuario", tr("Name"), &nome));
        let quando = gtk::MenuButton::new();
        quando.add_css_class("novo-agente-quando");
        let quando_rotulo = rotulo("", "");
        quando.set_child(Some(&quando_rotulo));
        quando.set_always_show_arrow(false);
        card.append(&campo("relogio", tr("When it runs"), &quando));
        let modelos = crate::configuracoes_agentes::text_models();
        let nomes: Vec<String> = modelos.iter().map(|p| format!("{} · {}", tr("Local"), crate::configuracoes_agentes::model_name(p))).collect();
        let nomes_ref: Vec<&str> = nomes.iter().map(String::as_str).collect();
        let modelo = gtk::DropDown::from_strings(&nomes_ref);
        modelo.add_css_class("novo-agente-modelo");
        // (the model the agent uses now first)
        let atual = std::env::var_os("KALANGO_MODELO").map(PathBuf::from)
            .or_else(|| crate::configuracoes::ajustes()["modelo_padrao"].as_str().map(PathBuf::from));
        if let Some(i) = atual.and_then(|a| modelos.iter().position(|p| *p == a)) {
            modelo.set_selected(i as u32);
        }
        modelo.set_sensitive(!modelos.is_empty());
        card.append(&campo("cpu", tr("Where it thinks"), &modelo));
        let seg = caixa(Orientation::Horizontal, 2, "cfg-segmentado");
        seg.add_css_class("escuro");
        seg.set_homogeneous(true);
        let mut autonomia: Vec<(&'static str, gtk::ToggleButton)> = Vec::new();
        for (id, n) in [("sempre", tr("Always ask")), ("risco", tr("Only when risky")), ("autonomo", tr("Autonomous"))] {
            let b = gtk::ToggleButton::with_label(n);
            b.add_css_class("cfg-seg");
            if let Some((_, f)) = autonomia.first() {
                b.set_group(Some(f));
            }
            seg.append(&b);
            crate::central::posicao_debug_sempre(&format!("novo agente autonomia {id}"), b.upcast_ref());
            autonomia.push((id, b));
        }
        card.append(&campo("escudo", tr("Autonomy"), &seg));
        entendeu.append(&card);
        let passos_secao = secao(tr("STEPS"), "lista");
        entendeu.append(&passos_secao);
        let passos_box = caixa(Orientation::Vertical, 0, "central-cartao");
        entendeu.append(&passos_box);
        let adicionar = gtk::Button::new();
        adicionar.add_css_class("central-adicionar");
        let ab = caixa(Orientation::Horizontal, 8, "");
        ab.append(&ui_icon("mais", "#C9CAD1", 13));
        ab.append(&rotulo(tr("Add step"), ""));
        adicionar.set_child(Some(&ab));
        adicionar.set_halign(Align::Start);
        adicionar.set_margin_top(10);
        entendeu.append(&adicionar);
        entendeu.set_visible(false);
        esq.append(&entendeu);
        let rol = gtk::ScrolledWindow::new();
        rol.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        rol.set_hexpand(true);
        rol.set_child(Some(&esq));
        raiz.append(&rol);

        // ---- right: permissions, the simulation, and creating it
        let lado = caixa(Orientation::Vertical, 0, "novo-agente-lado");
        lado.set_size_request(380, -1);
        let lado_conteudo = caixa(Orientation::Vertical, 0, "");
        lado_conteudo.append(&secao(tr("PERMISSIONS IT NEEDS"), "escudo"));
        let permissoes_box = caixa(Orientation::Vertical, 0, "central-cartao");
        lado_conteudo.append(&permissoes_box);
        lado_conteudo.append(&secao(tr("SIMULATION — IF IT RAN NOW"), "tocar"));
        let simulacao_box = caixa(Orientation::Vertical, 6, "novo-agente-simulacao");
        lado_conteudo.append(&simulacao_box);
        lado_conteudo.set_visible(false);
        let lrol = gtk::ScrolledWindow::new();
        lrol.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        lrol.set_vexpand(true);
        lrol.set_child(Some(&lado_conteudo));
        lado.append(&lrol);
        let nota = rotulo(tr("Once created, everything it does shows in the Timeline and can be undone."), "central-agente-sub");
        nota.set_xalign(0.0);
        nota.set_wrap(true);
        nota.set_margin_bottom(10);
        lado.append(&nota);
        let acoes = caixa(Orientation::Horizontal, 8, "");
        acoes.set_homogeneous(true);
        let rascunho = gtk::Button::with_label(tr("Save draft"));
        rascunho.add_css_class("novo-agente-rascunho");
        let criar = gtk::Button::new();
        criar.add_css_class("botao-principal");
        let cc = caixa(Orientation::Horizontal, 6, "");
        cc.set_halign(Align::Center);
        cc.append(&estrela(12, "#1C1D24"));
        cc.append(&rotulo(tr("Create agent"), ""));
        criar.set_child(Some(&cc));
        for b in [&rascunho, &criar] {
            b.set_sensitive(false);
            acoes.append(b);
        }
        lado.append(&acoes);
        raiz.append(&lado);
        crate::central::posicao_debug_sempre("novo agente gerar", gerar.upcast_ref());
        crate::central::posicao_debug_sempre("novo agente criar", criar.upcast_ref());
        crate::central::posicao_debug_sempre("novo agente rascunho", rascunho.upcast_ref());

        let p = Rc::new(NewAgentPage {
            raiz, texto: texto.clone(), gerar: gerar.clone(), estado, entendeu, nome, quando, quando_rotulo, modelo, modelos,
            autonomia, passos_box, passos_secao, permissoes_box, simulacao_box, lado: lado_conteudo, criar: criar.clone(),
            rascunho: rascunho.clone(), spec: RefCell::default(), desenhado: RefCell::default(), ler: Cell::new(true), mudar: Cell::new(true),
            geracao: Cell::new(0), preenchendo: Cell::new(false), ir: RefCell::default(),
        });
        // the placeholder while empty
        let d = dica.clone();
        texto.buffer().connect_changed(move |b| d.set_visible(b.char_count() == 0));
        // Enter puts it together (Shift+Enter: a new line)
        let teclas = gtk::EventControllerKey::new();
        let w = Rc::downgrade(&p);
        teclas.connect_key_pressed(move |_, k, _, m| {
            if matches!(k, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter) && !m.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
                if let Some(p) = w.upgrade() {
                    p.generate();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        texto.add_controller(teclas);
        let w = Rc::downgrade(&p);
        gerar.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.generate();
            }
        });
        // templates
        let menu = gtk::Popover::new();
        menu.add_css_class("menu");
        let lista = caixa(Orientation::Vertical, 0, "");
        for t in TEMPLATES {
            let b = gtk::Button::new();
            b.add_css_class("cmd-filtro-opcao");
            let l = rotulo(t, "");
            l.set_xalign(0.0);
            l.set_wrap(true);
            l.set_max_width_chars(48);
            b.set_child(Some(&l));
            let (w, m) = (Rc::downgrade(&p), menu.downgrade());
            b.connect_clicked(move |_| {
                if let Some(m) = m.upgrade() {
                    m.popdown();
                }
                if let Some(p) = w.upgrade() {
                    p.texto.buffer().set_text(t);
                    p.generate();
                }
            });
            lista.append(&b);
        }
        menu.set_child(Some(&lista));
        modelo_b.set_popover(Some(&menu));
        // an example file goes with the sentence (its name; the agent reads it when it runs)
        let w = Rc::downgrade(&p);
        exemplo.connect_clicked(move |_| {
            let w = w.clone();
            crate::arquivos::seletor::abrir_arquivo(tr("Agents"), tr("Attach an example"), None, Vec::new(), None, move |f| {
                if let Some(p) = w.upgrade() {
                    let b = p.texto.buffer();
                    let mut fim = b.end_iter();
                    b.insert(&mut fim, &format!(" ({})", trf("example: {path}", &[("path", f.display().to_string())])));
                }
            });
        });
        // the person's edits
        let w = Rc::downgrade(&p);
        p.nome.connect_changed(move |e| {
            if let Some(p) = w.upgrade().filter(|p| !p.preenchendo.get()) {
                if let Some(s) = p.spec.borrow_mut().as_mut() {
                    s.nome = e.text().to_string();
                }
                p.refresh_buttons();
            }
        });
        for (id, b) in &p.autonomia {
            let (w, id) = (Rc::downgrade(&p), *id);
            b.connect_toggled(move |b| {
                if let Some(p) = w.upgrade().filter(|p| b.is_active() && !p.preenchendo.get()) {
                    if let Some(s) = p.spec.borrow_mut().as_mut() {
                        s.autonomia = id.to_string();
                    }
                    p.render_derived(false);
                    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                        eprintln!("debug novo agente: autonomia {id}");
                    }
                }
            });
        }
        let w = Rc::downgrade(&p);
        adicionar.connect_clicked(move |b| {
            if let Some(p) = w.upgrade() {
                p.ask_step(b);
            }
        });
        let w = Rc::downgrade(&p);
        criar.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.keep(true);
            }
        });
        let w = Rc::downgrade(&p);
        rascunho.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.keep(false);
            }
        });
        p.quando.set_popover(Some(&p.when_editor()));
        p
    }

    /// Back to an empty page (opened again from "criar agente").
    pub fn reset(&self) {
        self.texto.buffer().set_text("");
        *self.spec.borrow_mut() = None;
        *self.desenhado.borrow_mut() = Default::default();
        self.entendeu.set_visible(false);
        self.lado.set_visible(false);
        self.estado.set_visible(false);
        self.refresh_buttons();
        self.texto.grab_focus();
    }

    /// The local model puts the agent together from the sentence (off the interface thread).
    fn generate(self: &Rc<Self>) {
        let b = self.texto.buffer();
        let frase = b.text(&b.start_iter(), &b.end_iter(), false).trim().to_string();
        if frase.is_empty() {
            return;
        }
        let g = self.geracao.get() + 1;
        self.geracao.set(g);
        self.estado.set_text(tr("The local model is putting the agent together…"));
        self.estado.set_visible(true);
        self.gerar.set_sensitive(false);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug novo agente: gerando «{frase}»");
        }
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || -> Result<Value, String> {
            use std::io::Write;
            let mut c = std::process::Command::new(crate::voz::agent_binary()).arg("agente-de-descricao").stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().map_err(|e| e.to_string())?;
            c.stdin.as_mut().ok_or("stdin")?.write_all(frase.as_bytes()).map_err(|e| e.to_string())?;
            drop(c.stdin.take());
            let o = c.wait_with_output().map_err(|e| e.to_string())?;
            serde_json::from_slice::<Value>(&o.stdout).map_err(|_| tr("The local model did not answer.").to_string())
        }, move |r| {
            let Some(p) = w.upgrade() else { return };
            if p.geracao.get() != g {
                return;
            }
            p.gerar.set_sensitive(true);
            match r {
                Ok(v) => {
                    p.estado.set_visible(false);
                    let s = Spec::from_json(&v);
                    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                        eprintln!("debug novo agente: «{}» · {} · {} passos ({}) · {}", s.nome, s.gatilho().texto(), s.passos.len(),
                            s.passos.iter().map(|x| x.acao.as_str()).collect::<Vec<_>>().join(", "), s.autonomia);
                    }
                    *p.spec.borrow_mut() = Some(s);
                    p.render();
                }
                Err(e) => p.estado.set_text(&e),
            }
        });
    }

    /// Everything from the spec: the understood card, the steps, and what depends on them.
    fn render(self: &Rc<Self>) {
        let Some(s) = self.spec.borrow().clone() else { return };
        self.preenchendo.set(true);
        self.nome.set_text(&s.nome);
        self.quando_rotulo.set_text(&s.gatilho().texto());
        for (id, b) in &self.autonomia {
            b.set_active(*id == s.autonomia);
        }
        self.preenchendo.set(false);
        // (the rows below come in one after the other: the cascade)
        self.entendeu.set_visible(true);
        self.lado.set_visible(true);
        self.render_derived(true);
        self.refresh_buttons();
    }

    /// The steps, the permissions and the simulation (again when the autonomy or the steps change). Only the rows
    /// that changed come in again.
    fn render_derived(self: &Rc<Self>, cascade: bool) {
        let Some(s) = self.spec.borrow().clone() else { return };
        // steps
        let antes = self.desenhado.borrow().clone();
        let mut novos_passos = Vec::new();
        clear(&self.passos_box);
        let mut anims = Vec::new();
        for (i, p) in s.passos.iter().enumerate() {
            let chave = format!("{}|{}|{}", p.texto, p.acao, asks(&p.acao, &s.autonomia));
            let row = caixa(Orientation::Horizontal, 10, "novo-agente-passo");
            let n = rotulo(&(i + 1).to_string(), "novo-agente-numero");
            n.set_valign(Align::Center);
            row.append(&n);
            let t = rotulo(&p.texto, "central-limite-nome");
            t.set_xalign(0.0);
            t.set_hexpand(true);
            t.set_wrap(true);
            row.append(&t);
            let (tag, cl) = match p.acao.as_str() {
                "apagar" if asks("apagar", &s.autonomia) => (tr("delete · asks first").to_string(), "ouro"),
                "apagar" => (tr("delete").to_string(), "coral"),
                "mover" => (tr("move").to_string(), "verde"),
                "escrever" => (tr("write").to_string(), "verde"),
                "notificar" => (tr("notify").to_string(), "azul"),
                "rede" => (if asks("rede", &s.autonomia) { tr("network · asks first") } else { tr("network") }.to_string(), "ouro"),
                "terminal" => (if asks("terminal", &s.autonomia) { tr("program · asks first") } else { tr("program") }.to_string(), "ouro"),
                _ => (tr("read").to_string(), "cinza"),
            };
            let chip = rotulo(&tag, "novo-agente-tag");
            chip.add_css_class(cl);
            chip.set_valign(Align::Center);
            row.append(&chip);
            let tirar = gtk::Button::new();
            tirar.add_css_class("novo-agente-tirar");
            tirar.set_child(Some(&ui_icon("fechar", "#8E8F99", 10)));
            tirar.set_tooltip_text(Some(tr("Remove this step")));
            tirar.set_valign(Align::Center);
            let w = Rc::downgrade(self);
            tirar.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    if let Some(s) = p.spec.borrow_mut().as_mut() {
                        if i < s.passos.len() {
                            s.passos.remove(i);
                        }
                    }
                    p.render_derived(false);
                    p.refresh_buttons();
                }
            });
            row.append(&tirar);
            let a = Animavel::new(&row);
            self.passos_box.append(&a);
            if cascade || !antes.0.contains(&chave) {
                anims.push(a);
            }
            novos_passos.push(chave);
        }
        self.passos_secao.set_visible(true);
        // permissions: the minimal ones for these steps
        clear(&self.permissoes_box);
        let mut novas_perm = Vec::new();
        let pastas: Vec<String> = {
            let mut v: Vec<String> = s.passos.iter().map(|p| p.pasta.trim().to_string()).filter(|p| !p.is_empty()).collect();
            v.dedup();
            v
        };
        let quais = |v: &[String]| match v {
            [] => tr("the folders its steps name").to_string(),
            [a] => a.clone(),
            [a @ .., b] => trf("{list} and {last}", &[("list", a.join(", ")), ("last", b.clone())]),
        };
        let precisa = |a: &[&str]| s.passos.iter().any(|p| a.contains(&p.acao.as_str()));
        let mut linhas: Vec<(String, String, String, Option<(bool, &'static str)>, Option<String>)> = Vec::new();
        linhas.push(("olho".into(), trf("Read {folders}", &[("folders", quais(&pastas))]), String::new(), Some((self.ler.get(), "ler")), None));
        if precisa(&["mover", "escrever"]) {
            let sub = trf("inside {folders}", &[("folders", quais(&pastas))]);
            linhas.push(("pasta".into(), if precisa(&["mover"]) { tr("Move files") } else { tr("Write files") }.to_string(), sub, Some((self.mudar.get(), "mudar")), None));
        }
        if precisa(&["apagar"]) {
            let chip = if asks("apagar", &s.autonomia) { tr("ask") } else { tr("free") };
            linhas.push(("lixeira".into(), tr("Delete files").to_string(), tr("they go to the trash for 30 days").to_string(), None, Some(chip.to_string())));
        }
        if precisa(&["terminal"]) {
            let chip = if asks("terminal", &s.autonomia) { tr("ask") } else { tr("free") };
            linhas.push(("terminal".into(), tr("Run programs").to_string(), String::new(), None, Some(chip.to_string())));
        }
        let rede = if precisa(&["rede"]) { if asks("rede", &s.autonomia) { tr("ask") } else { tr("free") } } else { tr("none") };
        linhas.push(("rede-cabo".into(), tr("Network").to_string(), String::new(), None, Some(rede.to_string())));
        let mut anims_perm = Vec::new();
        for (icone, titulo, sub, interruptor, chip) in linhas {
            let chave = format!("{titulo}|{sub}|{chip:?}|{:?}", interruptor.map(|x| x.0));
            let r = caixa(Orientation::Horizontal, 10, "novo-agente-permissao");
            r.append(&ui_icon(&icone, "#8E8F99", 15));
            let col = caixa(Orientation::Vertical, 1, "");
            col.set_hexpand(true);
            col.set_valign(Align::Center);
            let tl = rotulo(&titulo, "central-limite-nome");
            tl.set_xalign(0.0);
            tl.set_ellipsize(gtk::pango::EllipsizeMode::End);
            tl.set_max_width_chars(1);
            col.append(&tl);
            if !sub.is_empty() {
                let sl = rotulo(&sub, "central-agente-sub");
                sl.set_xalign(0.0);
                sl.set_ellipsize(gtk::pango::EllipsizeMode::End);
                sl.set_max_width_chars(1);
                col.append(&sl);
            }
            r.append(&col);
            if let Some((on, qual)) = interruptor {
                let sw = gtk::Switch::new();
                sw.add_css_class("central-switch");
                sw.set_valign(Align::Center);
                sw.set_active(on);
                let w = Rc::downgrade(self);
                sw.connect_state_set(move |_, v| {
                    if let Some(p) = w.upgrade() {
                        if qual == "ler" { p.ler.set(v) } else { p.mudar.set(v) }
                        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                            eprintln!("debug novo agente: {qual} {v}");
                        }
                    }
                    glib::Propagation::Proceed
                });
                crate::central::posicao_debug(&format!("novo agente permissao {qual}"), sw.upcast_ref());
                r.append(&sw);
            }
            if let Some(c) = chip {
                let l = rotulo(&c, "novo-agente-tag");
                l.add_css_class(if c == tr("none") { "verde" } else if c == tr("free") { "coral" } else { "ouro" });
                l.set_valign(Align::Center);
                r.append(&l);
            }
            let a = Animavel::new(&r);
            self.permissoes_box.append(&a);
            if cascade || !antes.1.contains(&chave) {
                anims_perm.push(a);
            }
            novas_perm.push(chave);
        }
        mv::cascata(&anims, 6.0);
        mv::cascata(&anims_perm, 6.0);
        self.desenhado.borrow_mut().0 = novos_passos;
        self.desenhado.borrow_mut().1 = novas_perm;
        // the simulation: counted in the folders (off the interface thread)
        let (passos, autonomia, g) = (s.passos.clone(), s.autonomia.clone(), self.geracao.get());
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || simulate(&passos, &autonomia), move |linhas| {
            let Some(p) = w.upgrade() else { return };
            if p.geracao.get() != g {
                return;
            }
            p.show_simulation(&linhas, cascade);
        });
    }

    fn show_simulation(&self, linhas: &[(bool, String)], cascade: bool) {
        let antes = self.desenhado.borrow().2.clone();
        clear(&self.simulacao_box);
        let mut anims = Vec::new();
        let mut novas = Vec::new();
        for (pede, t) in linhas {
            let r = caixa(Orientation::Horizontal, 8, "");
            r.append(&ui_icon(if *pede { "alerta" } else { "check" }, if *pede { "#E5C98C" } else { "#8ED2C3" }, 13));
            let l = rotulo(t, "novo-agente-sim");
            l.set_xalign(0.0);
            l.set_wrap(true);
            l.set_hexpand(true);
            r.append(&l);
            let a = Animavel::new(&r);
            self.simulacao_box.append(&a);
            if cascade || !antes.contains(t) {
                anims.push(a);
            }
            novas.push(t.clone());
        }
        let nada = rotulo(tr("Nothing was changed: this is only the preview."), "central-agente-sub");
        nada.set_xalign(0.0);
        nada.set_margin_top(4);
        self.simulacao_box.append(&nada);
        mv::cascata(&anims, 6.0);
        self.desenhado.borrow_mut().2 = novas;
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug novo agente: simulação · {}", linhas.iter().map(|(p, t)| format!("{}{t}", if *p { "⚠ " } else { "" })).collect::<Vec<_>>().join(" · "));
        }
    }

    /// "Adicionar passo": a line to write it; its kind comes from its words, its folder from the step before.
    fn ask_step(self: &Rc<Self>, b: &gtk::Button) {
        let pop = gtk::Popover::new();
        pop.set_parent(b);
        pop.set_position(gtk::PositionType::Bottom);
        let e = gtk::Entry::new();
        e.set_placeholder_text(Some(tr("What else should it do?")));
        e.set_width_chars(36);
        pop.set_child(Some(&e));
        let (w, pw) = (Rc::downgrade(self), pop.downgrade());
        e.connect_activate(move |e| {
            let t = e.text().trim().to_string();
            if let Some(pop) = pw.upgrade() {
                pop.popdown();
            }
            let Some(p) = w.upgrade() else { return };
            if t.is_empty() {
                return;
            }
            if let Some(s) = p.spec.borrow_mut().as_mut() {
                let pasta = s.passos.iter().rev().find(|x| !x.pasta.is_empty()).map(|x| x.pasta.clone()).unwrap_or_default();
                s.passos.push(Step { acao: guess_action(&t).to_string(), texto: t, pasta, tipos: Vec::new(), idade_dias: 0 });
            }
            p.render_derived(false);
            p.refresh_buttons();
        });
        pop.connect_closed(|p| p.unparent());
        pop.popup();
        e.grab_focus();
    }

    /// When it runs: the days, the time, or only when the person asks.
    fn when_editor(self: &Rc<Self>) -> gtk::Popover {
        let pop = gtk::Popover::new();
        let col = caixa(Orientation::Vertical, 10, "novo-agente-quando-editor");
        let dias = caixa(Orientation::Horizontal, 4, "");
        let mut botoes = Vec::new();
        for d in 1..=7u8 {
            let nome = glib::DateTime::from_local(2024, 1, d as i32, 12, 0, 0.0).ok().and_then(|x| x.format("%a").ok()).map(|s| s.trim_end_matches('.').to_string()).unwrap_or_default();
            let b = gtk::ToggleButton::with_label(&nome);
            b.add_css_class("cfg-seg");
            dias.append(&b);
            botoes.push((d, b));
        }
        col.append(&dias);
        let hora = caixa(Orientation::Horizontal, 6, "");
        let h = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
        let m = gtk::SpinButton::with_range(0.0, 55.0, 5.0);
        hora.append(&rotulo(tr("at"), "central-agente-sub"));
        hora.append(&h);
        hora.append(&rotulo(":", ""));
        hora.append(&m);
        col.append(&hora);
        let manual = gtk::CheckButton::with_label(tr("Only when I ask"));
        col.append(&manual);
        pop.set_child(Some(&col));
        // (filled from the spec when it opens; applied when it closes)
        let (w, bs, hh, mm, ma) = (Rc::downgrade(self), botoes.clone(), h.clone(), m.clone(), manual.clone());
        pop.connect_show(move |_| {
            let Some(p) = w.upgrade() else { return };
            let q = p.spec.borrow().as_ref().and_then(|s| s.quando.clone());
            ma.set_active(q.is_none());
            let (ds, hr, mi) = q.unwrap_or((Vec::new(), 18, 0));
            for (d, b) in &bs {
                b.set_active(ds.contains(d));
            }
            hh.set_value(hr as f64);
            mm.set_value(mi as f64);
        });
        let w = Rc::downgrade(self);
        pop.connect_closed(move |_| {
            let Some(p) = w.upgrade() else { return };
            let q = (!manual.is_active()).then(|| (botoes.iter().filter(|(_, b)| b.is_active()).map(|(d, _)| *d).collect::<Vec<u8>>(), h.value() as u8, m.value() as u8));
            if let Some(s) = p.spec.borrow_mut().as_mut() {
                s.quando = q;
            }
            let t = p.spec.borrow().as_ref().map(|s| s.gatilho().texto()).unwrap_or_default();
            p.quando_rotulo.set_text(&t);
        });
        pop
    }

    fn refresh_buttons(&self) {
        let ok = self.spec.borrow().as_ref().is_some_and(|s| !s.passos.is_empty() && !self.nome.text().trim().is_empty());
        self.criar.set_sensitive(ok);
        self.rascunho.set_sensitive(ok);
    }

    /// Kept as a routine: on ("Criar agente") or paused, a draft ("Salvar rascunho").
    fn keep(self: &Rc<Self>, ativa: bool) {
        let Some(s) = self.spec.borrow().clone() else { return };
        let b = self.texto.buffer();
        let descricao = b.text(&b.start_iter(), &b.end_iter(), false).trim().to_string();
        let nome = self.nome.text().trim().to_string();
        let modelo = self.modelos.get(self.modelo.selected() as usize).filter(|_| self.modelo.is_sensitive());
        let atual = std::env::var_os("KALANGO_MODELO").map(PathBuf::from)
            .or_else(|| crate::configuracoes::ajustes()["modelo_padrao"].as_str().map(PathBuf::from));
        let cfg = AgenteConfig {
            descricao,
            passos: s.passos.iter().map(|p| (p.texto.clone(), p.acao.clone())).collect(),
            autonomia: s.autonomia.clone(),
            desligadas: columns_off(&s.passos, self.ler.get(), self.mudar.get()),
            // (empty: the model the agent uses anyway)
            modelo: modelo.filter(|m| Some(*m) != atual.as_ref()).map(|m| m.display().to_string()).unwrap_or_default(),
            ..Default::default()
        };
        let pedido = cfg.pedido(&nome);
        let mut r = Rotinas::carregar();
        let id = r.adicionar(&nome, Acao::Pedido(pedido), s.gatilho());
        if let Some(x) = r.lista.iter_mut().find(|x| x.id == id) {
            x.ativa = ativa;
            x.agente = Some(cfg.clone());
        }
        r.salvar();
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug novo agente: {} «{nome}» ({}, sem {})", if ativa { "criado" } else { "rascunho" }, s.gatilho().texto(), cfg.desligadas.join(", "));
        }
        if let Some(f) = self.ir.borrow().as_ref() {
            f("rotinas");
        }
    }
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passo(acao: &str, pasta: &str, tipos: &[&str], idade: u64) -> Step {
        Step { texto: acao.into(), acao: acao.into(), pasta: pasta.into(), tipos: tipos.iter().map(|t| t.to_string()).collect(), idade_dias: idade }
    }

    #[test]
    fn what_asks_first() {
        assert!(asks("apagar", "risco") && !asks("mover", "risco") && asks("rede", "risco"));
        assert!(asks("mover", "sempre") && !asks("ler", "sempre") && !asks("notificar", "sempre"));
        assert!(!asks("apagar", "autonomo"));
    }

    #[test]
    fn only_the_columns_its_steps_need() {
        let p = vec![passo("mover", "~/Downloads", &[], 0), passo("apagar", "~/Downloads", &["deb"], 30), passo("notificar", "", &[], 0)];
        assert_eq!(columns_off(&p, true, true), vec!["rede", "terminal", "tela", "gastar"]);
        assert_eq!(columns_off(&p, true, false), vec!["escrever", "rede", "terminal", "tela", "gastar"]);
        assert_eq!(columns_off(&[passo("ler", "~", &[], 0)], true, true), vec!["escrever", "apagar", "rede", "terminal", "tela", "gastar"]);
    }

    #[test]
    fn the_simulation_counts_and_changes_nothing() {
        let d = std::env::temp_dir().join(format!("kalango-novo-agente-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for (n, bytes) in [("a.pdf", 10), ("b.png", 20), ("c.deb", 1000), ("d.AppImage", 3000), (".oculto", 5)] {
            std::fs::write(d.join(n), vec![0u8; bytes]).unwrap();
        }
        let pasta = d.display().to_string();
        let p = vec![passo("mover", &pasta, &[], 0), passo("apagar", &pasta, &["deb", "appimage"], 0), passo("notificar", "", &[], 0)];
        let s = simulate(&p, "risco");
        assert_eq!(s[0], (false, trf("would move {n} files into {k} folders", &[("n", "4".into()), ("k", "3".into())])));
        assert!(s[1].0, "deleting asks first");
        assert_eq!(s[1].1, trf("would ask to delete {n} files ({size})", &[("n", "2".into()), ("size", glib::format_size(4000).to_string())]));
        // (older than 30 days: none — they were just made)
        let s = simulate(&[passo("apagar", &pasta, &["deb"], 30)], "autonomo");
        assert_eq!(s[0].1, trf("would delete nothing now: no file in {folder} matches", &[("folder", pasta.clone())]));
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 5, "nothing changed");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_step_added_by_hand_gets_its_kind() {
        assert_eq!(guess_action("Apagar os temporários"), "apagar");
        assert_eq!(guess_action("Separar por tipo"), "mover");
        assert_eq!(guess_action("Me mandar um resumo"), "notificar");
        assert_eq!(guess_action("Conferir a pasta"), "ler");
    }
}
