//! Agents center › Ajustes › Adicionar conector (design "Ajustes · Adicionar conector (MCP)"): nothing is added before
//! the test — kalangod starts the connector isolated, lists what it offers, and each tool's permission can be set
//! before adding. Three ways in: the catalog shipped with the system, a configuration pasted from the service's docs
//! (`{"mcpServers": …}`), or by hand (name, command and arguments or an address, environment variables, isolation).
//!
//! Environment values go to the system vault (`kalangod cofre guardar`), never into the configuration file: the
//! manifest says `cofre:NAME` and kalangod gives the value only to that connector, when it starts.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, estrela, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;

const CATALOGO: &str = include_str!("../assets/conectores-catalogo.json");

/// Where the connectors shipped with KalangoOS are (installed, or this repository in development).
fn conectores_do_sistema() -> PathBuf {
    let instalado = PathBuf::from("/usr/lib/kalango/conectores");
    if instalado.is_dir() {
        return instalado;
    }
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../sessao/ferramentas")).canonicalize().unwrap_or_default()
}

fn which(p: &str) -> Option<String> {
    if p.contains('/') {
        return Some(p.to_string());
    }
    std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|d| d.join(p)).find(|x| x.is_file())).map(|x| x.display().to_string())
}

/// The command's program is a package runner that fetches community code (npx, uvx…).
fn de_comunidade(comando: &str) -> bool {
    let base = std::path::Path::new(comando).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    matches!(base.as_str(), "npx" | "uvx" | "pipx" | "bunx" | "pnpx")
}

/// A vault name for a connector's variable (only letters, digits, ".", "-").
fn nome_no_cofre(conector: &str, var: &str) -> String {
    format!("{conector}.{}", var.to_lowercase().replace('_', "-"))
}

/// Called when the page closes: Some(name) when a connector was added.
type AoVoltar = Box<dyn Fn(Option<String>)>;

/// A value as the services write it in their docs: strings quoted, lists and maps on one line with ", " and ": ".
fn compacto(v: &Value) -> String {
    match v {
        Value::Array(a) => format!("[{}]", a.iter().map(compacto).collect::<Vec<_>>().join(", ")),
        Value::Object(o) => format!("{{ {} }}", o.iter().map(|(k, x)| format!("{}: {}", json!(k), compacto(x))).collect::<Vec<_>>().join(", ")),
        x => x.to_string(),
    }
}

pub struct NovoConector {
    pub raiz: gtk::Box,
    ao_voltar: RefCell<Option<AoVoltar>>,
    abas: Vec<gtk::ToggleButton>,
    paginas: gtk::Stack,
    primeiro_do_catalogo: Option<gtk::Button>,
    // by hand
    nome: gtk::Entry,
    local: gtk::ToggleButton,
    remoto: gtk::ToggleButton,
    linha_local: gtk::Box,
    comando: gtk::Entry,
    argumentos: gtk::Entry,
    url: gtk::Entry,
    variaveis: gtk::Box,
    vars: RefCell<Vec<(gtk::Entry, gtk::PasswordEntry, gtk::Box)>>,
    sem_rede: gtk::Switch,
    pastas: gtk::DropDown,
    pasta_escolhida: RefCell<Option<PathBuf>>,
    /// the program's own folder (read and run): from the catalog, or the folder of a script given as an argument
    pasta_programa: RefCell<Option<String>>,
    sob_demanda: gtk::Switch,
    json_texto: gtk::Label,
    colar: gtk::TextView,
    colar_erro: gtk::Label,
    // the test, on the right
    passos: gtk::Box,
    ferramentas: gtk::Box,
    aviso: gtk::Box,
    aviso_texto: gtk::Label,
    adicionar: gtk::Button,
    resultado: RefCell<Option<Result<Value, String>>>,
    perms: RefCell<BTreeMap<String, String>>,
    espera: RefCell<Option<glib::SourceId>>,
    geracao: Cell<u64>,
    preenchendo: Cell<bool>,
}

impl NovoConector {
    pub fn new(compacta: bool) -> Rc<NovoConector> {
        let raiz = caixa(Orientation::Horizontal, 0, "");
        raiz.set_vexpand(true);
        let corpo = caixa(Orientation::Vertical, 0, "central-corpo");
        corpo.set_hexpand(true);
        let voltar = gtk::Button::new();
        voltar.add_css_class("rotinas-texto");
        let vc = caixa(Orientation::Horizontal, 6, "");
        vc.append(&ui_icon("seta-esquerda", "#C9CAD1", 13));
        vc.append(&rotulo(tr("Connectors (MCP)"), ""));
        voltar.set_child(Some(&vc));
        voltar.set_halign(Align::Start);
        corpo.append(&voltar);
        let h = rotulo(tr("Add connector"), "central-titulo");
        h.set_xalign(0.0);
        h.set_margin_top(6);
        corpo.append(&h);
        let sub = rotulo(tr("Pick one from the catalog, paste the configuration the service gives you, or fill it in by hand."), "central-sub");
        sub.set_xalign(0.0);
        sub.set_wrap(true);
        sub.set_margin_top(4);
        corpo.append(&sub);
        let seg = caixa(Orientation::Horizontal, 0, "uso-segmentos");
        seg.set_homogeneous(true);
        seg.set_margin_top(14);
        let mut abas: Vec<gtk::ToggleButton> = Vec::new();
        for t in [tr("Catalog"), tr("Paste configuration"), tr("By hand")] {
            let b = gtk::ToggleButton::with_label(t);
            b.add_css_class("uso-segmento");
            if let Some(f) = abas.first() {
                b.set_group(Some(f));
            }
            seg.append(&b);
            abas.push(b);
        }
        corpo.append(&seg);
        let paginas = gtk::Stack::new();
        paginas.set_vhomogeneous(false);
        paginas.set_margin_top(12);
        // ---- catalog
        let cat = caixa(Orientation::Vertical, 8, "");
        let itens: Vec<Value> = serde_json::from_str(CATALOGO).unwrap_or_default();
        let cat_botoes: Vec<(gtk::Button, Value)> = itens.into_iter().map(|it| {
            let b = gtk::Button::new();
            b.add_css_class("ajustes-catalogo");
            let l = caixa(Orientation::Horizontal, 12, "");
            let ic = caixa(Orientation::Horizontal, 0, "rotinas-icone");
            ic.set_size_request(36, 36);
            ic.set_valign(Align::Center);
            ic.set_hexpand(false);
            let g = ui_icon(it["glifo"].as_str().unwrap_or("cabo"), "#C9CAD1", 16);
            g.set_hexpand(true);
            g.set_halign(Align::Center);
            ic.append(&g);
            l.append(&ic);
            let c = caixa(Orientation::Vertical, 2, "");
            c.set_hexpand(true);
            let t = rotulo(it["titulo"].as_str().unwrap_or(""), "rotinas-nome");
            t.set_xalign(0.0);
            c.append(&t);
            let d = rotulo(it["descricao"].as_str().unwrap_or(""), "central-agente-sub");
            d.set_xalign(0.0);
            d.set_wrap(true);
            c.append(&d);
            l.append(&c);
            let tipo = rotulo(&if it["url"].is_string() { tr("remote · HTTP").to_string() } else { it["comando"].as_str().unwrap_or("").to_string() }, "ajustes-chip");
            tipo.set_valign(Align::Center);
            l.append(&tipo);
            b.set_child(Some(&l));
            cat.append(&b);
            (b, it)
        }).collect();
        let cat_rol = gtk::ScrolledWindow::new();
        cat_rol.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        cat_rol.set_vexpand(true);
        cat_rol.set_child(Some(&cat));
        paginas.add_named(&cat_rol, Some("catalogo"));
        // ---- paste
        let col = caixa(Orientation::Vertical, 10, "");
        let colar = gtk::TextView::new();
        colar.add_css_class("ajustes-colar");
        colar.set_monospace(true);
        colar.set_wrap_mode(gtk::WrapMode::WordChar);
        colar.set_size_request(-1, 220);
        colar.buffer().set_text("{\n  \"mcpServers\": {\n  }\n}");
        col.append(&colar);
        let colar_erro = rotulo("", "rotinas-erro");
        colar_erro.set_xalign(0.0);
        colar_erro.set_visible(false);
        col.append(&colar_erro);
        let usar = gtk::Button::with_label(tr("Use this configuration"));
        usar.add_css_class("rotinas-criar");
        usar.set_halign(Align::Start);
        col.append(&usar);
        paginas.add_named(&col, Some("colar"));
        // ---- by hand
        let man = caixa(Orientation::Vertical, 12, "");
        let form = caixa(Orientation::Vertical, 10, "rotinas-form");
        let rotulo_campo = |t: &str| {
            let l = rotulo(t, "central-agente-sub");
            l.set_xalign(0.0);
            l
        };
        let l1 = caixa(Orientation::Horizontal, 12, "");
        let c1 = caixa(Orientation::Vertical, 6, "");
        c1.set_hexpand(true);
        c1.append(&rotulo_campo(tr("Name")));
        let nome = gtk::Entry::new();
        nome.add_css_class("rotinas-campo");
        nome.set_placeholder_text(Some("postgres-local"));
        c1.append(&nome);
        l1.append(&c1);
        let c2 = caixa(Orientation::Vertical, 6, "");
        c2.append(&rotulo_campo(tr("Kind")));
        let tseg = caixa(Orientation::Horizontal, 0, "uso-segmentos");
        let local = gtk::ToggleButton::with_label(tr("Local · command"));
        local.add_css_class("uso-segmento");
        let remoto = gtk::ToggleButton::with_label(tr("Remote · URL"));
        remoto.add_css_class("uso-segmento");
        remoto.set_group(Some(&local));
        local.set_active(true);
        tseg.append(&local);
        tseg.append(&remoto);
        c2.append(&tseg);
        l1.append(&c2);
        form.append(&l1);
        let linha_local = caixa(Orientation::Horizontal, 12, "");
        let c3 = caixa(Orientation::Vertical, 6, "");
        c3.append(&rotulo_campo(tr("Command")));
        let comando = gtk::Entry::new();
        comando.add_css_class("rotinas-campo");
        comando.set_placeholder_text(Some("npx"));
        comando.set_width_chars(14);
        c3.append(&comando);
        linha_local.append(&c3);
        let c4 = caixa(Orientation::Vertical, 6, "");
        c4.set_hexpand(true);
        c4.append(&rotulo_campo(tr("Arguments")));
        let argumentos = gtk::Entry::new();
        argumentos.add_css_class("rotinas-campo");
        argumentos.set_placeholder_text(Some("-y @modelcontextprotocol/server-postgres"));
        c4.append(&argumentos);
        linha_local.append(&c4);
        form.append(&linha_local);
        let url = gtk::Entry::new();
        url.add_css_class("rotinas-campo");
        url.set_placeholder_text(Some("https://…/mcp"));
        url.set_visible(false);
        form.append(&url);
        form.append(&rotulo_campo(tr("Environment variables")));
        let variaveis = caixa(Orientation::Vertical, 6, "");
        form.append(&variaveis);
        let mais_var = gtk::Button::new();
        mais_var.add_css_class("central-adicionar");
        let mv = caixa(Orientation::Horizontal, 8, "");
        mv.append(&ui_icon("mais", "#C9CAD1", 13));
        mv.append(&rotulo(tr("Add variable"), ""));
        mais_var.set_child(Some(&mv));
        mais_var.set_halign(Align::Start);
        form.append(&mais_var);
        man.append(&form);
        // isolation
        let iso = caixa(Orientation::Vertical, 0, "central-cartao");
        let linha_iso = |glifo: &str, t: &str, controle: &gtk::Widget| -> gtk::Label {
            let l = caixa(Orientation::Horizontal, 12, "memoria-chave");
            l.append(&ui_icon(glifo, "#8E8F99", 16));
            let c = caixa(Orientation::Vertical, 1, "");
            c.set_hexpand(true);
            c.set_valign(Align::Center);
            let a = rotulo(t, "central-limite-nome");
            a.set_xalign(0.0);
            c.append(&a);
            let b = rotulo("", "central-agente-sub");
            b.set_xalign(0.0);
            c.append(&b);
            l.append(&c);
            controle.set_valign(Align::Center);
            l.append(controle);
            iso.append(&l);
            b
        };
        let sem_rede = gtk::Switch::new();
        sem_rede.add_css_class("central-switch");
        sem_rede.set_active(true);
        let rede_sub = linha_iso("wifi", tr("No access to the outside network"), sem_rede.upcast_ref());
        let pastas = gtk::DropDown::from_strings(&[tr("none"), tr("Documents (read only)"), tr("Downloads (read only)"), tr("Choose a folder…")]);
        pastas.add_css_class("central-escolha");
        pastas.set_valign(Align::Center);
        let pastas_sub = linha_iso("pasta", tr("Folders it can see"), pastas.upcast_ref());
        let sob_demanda = gtk::Switch::new();
        sob_demanda.add_css_class("central-switch");
        sob_demanda.set_active(true);
        linha_iso("desligar", tr("Start only when an agent needs it"), sob_demanda.upcast_ref());
        man.append(&iso);
        // the equivalent JSON
        let js = caixa(Orientation::Vertical, 8, "ajustes-registro");
        let jc = caixa(Orientation::Horizontal, 8, "");
        jc.append(&rotulo("JSON", "central-limite-nome"));
        let jsub = rotulo(tr("the same in mcp.json"), "central-agente-sub");
        jsub.set_hexpand(true);
        jsub.set_xalign(0.0);
        jc.append(&jsub);
        let copiar = gtk::Button::new();
        copiar.add_css_class("rotinas-texto");
        let cc = caixa(Orientation::Horizontal, 6, "");
        cc.append(&ui_icon("doc", "#8E8F99", 13));
        cc.append(&rotulo(tr("Copy"), ""));
        copiar.set_child(Some(&cc));
        jc.append(&copiar);
        js.append(&jc);
        let json_texto = rotulo("", "ajustes-log");
        json_texto.set_xalign(0.0);
        json_texto.set_selectable(true);
        js.append(&json_texto);
        man.append(&js);
        let man_rol = gtk::ScrolledWindow::new();
        man_rol.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        man_rol.set_vexpand(true);
        man_rol.set_child(Some(&man));
        paginas.add_named(&man_rol, Some("manual"));
        paginas.set_vexpand(true);
        corpo.append(&paginas);
        raiz.append(&corpo);

        // ---- the test, on the right
        let painel = caixa(Orientation::Vertical, 0, "central-painel");
        painel.set_size_request(if compacta { 340 } else { 474 }, -1);
        // fixed: what expands inside it must not widen it (GTK passes expand up to the ancestors)
        painel.set_hexpand(false);
        let cab = caixa(Orientation::Horizontal, 8, "");
        cab.append(&ui_icon("reiniciar", "#A5A1E2", 14));
        cab.append(&rotulo(tr("CONNECTION TEST"), "central-painel-agente"));
        painel.append(&cab);
        let passos = caixa(Orientation::Vertical, 8, "central-cartao");
        passos.add_css_class("memoria-info");
        passos.set_margin_top(12);
        painel.append(&passos);
        let ft = rotulo(tr("WHAT IT WILL BE ABLE TO DO — ADJUST BEFORE ADDING"), "central-painel-secao");
        ft.set_xalign(0.0);
        ft.set_wrap(true);
        ft.set_margin_top(18);
        painel.append(&ft);
        let ferramentas = caixa(Orientation::Vertical, 0, "central-cartao");
        ferramentas.set_margin_top(10);
        let frol = gtk::ScrolledWindow::new();
        frol.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        frol.set_vexpand(true);
        frol.set_child(Some(&ferramentas));
        painel.append(&frol);
        let aviso = caixa(Orientation::Vertical, 4, "ajustes-aviso");
        aviso.set_margin_top(12);
        let ac = caixa(Orientation::Horizontal, 8, "");
        ac.append(&ui_icon("alerta", "#E5C98C", 15));
        ac.append(&rotulo(tr("Community package, not audited"), "central-limite-nome"));
        aviso.append(&ac);
        let aviso_texto = rotulo("", "central-agente-sub");
        aviso_texto.set_xalign(0.0);
        aviso_texto.set_wrap(true);
        aviso.append(&aviso_texto);
        aviso.set_visible(false);
        painel.append(&aviso);
        let qp = rotulo(tr("WHO CAN USE IT"), "central-painel-secao");
        qp.set_xalign(0.0);
        qp.set_margin_top(14);
        painel.append(&qp);
        let quem = caixa(Orientation::Horizontal, 5, "ajustes-quem");
        quem.add_css_class("ativo");
        quem.set_halign(Align::Start);
        quem.set_margin_top(8);
        quem.append(&estrela(11, "#1C1D24"));
        quem.append(&rotulo(tr("Agent"), ""));
        painel.append(&quem);
        let bs = caixa(Orientation::Horizontal, 8, "");
        bs.set_homogeneous(true);
        bs.set_margin_top(16);
        let cancelar = gtk::Button::with_label(tr("Cancel"));
        cancelar.add_css_class("central-botao");
        bs.append(&cancelar);
        let adicionar = gtk::Button::new();
        adicionar.add_css_class("linha-desfazer");
        let adc = caixa(Orientation::Horizontal, 8, "");
        adc.set_halign(Align::Center);
        adc.append(&ui_icon("desligar", "#1C1D24", 14));
        adc.append(&rotulo(tr("Add connector"), ""));
        adicionar.set_child(Some(&adc));
        adicionar.set_sensitive(false);
        bs.append(&adicionar);
        painel.append(&bs);
        raiz.append(&painel);

        let n = Rc::new(NovoConector {
            primeiro_do_catalogo: cat_botoes.first().map(|(b, _)| b.clone()), raiz, ao_voltar: RefCell::default(), abas, paginas, nome, local, remoto, linha_local, comando, argumentos, url, variaveis, vars: RefCell::default(),
            sem_rede, pastas, pasta_escolhida: RefCell::default(), pasta_programa: RefCell::default(), sob_demanda, json_texto, colar, colar_erro, passos, ferramentas, aviso, aviso_texto, adicionar,
            resultado: RefCell::default(), perms: RefCell::default(), espera: RefCell::default(), geracao: Cell::new(0), preenchendo: Cell::new(false),
        });
        for (i, b) in n.abas.iter().enumerate() {
            let w = Rc::downgrade(&n);
            b.connect_toggled(move |b| {
                if let Some(n) = w.upgrade().filter(|_| b.is_active()) {
                    n.paginas.set_visible_child_name(["catalogo", "colar", "manual"][i]);
                }
            });
        }
        n.abas[2].set_active(true);
        for (b, it) in cat_botoes {
            let w = Rc::downgrade(&n);
            b.connect_clicked(move |_| {
                if let Some(n) = w.upgrade() {
                    n.do_catalogo(&it);
                }
            });
        }
        let w = Rc::downgrade(&n);
        usar.connect_clicked(move |_| {
            if let Some(n) = w.upgrade() {
                n.do_colado();
            }
        });
        let w = Rc::downgrade(&n);
        n.local.connect_toggled(move |b| {
            if let Some(n) = w.upgrade() {
                n.linha_local.set_visible(b.is_active());
                n.url.set_visible(!b.is_active());
                n.mudou();
            }
        });
        for e in [&n.nome, &n.comando, &n.argumentos, &n.url] {
            let w = Rc::downgrade(&n);
            e.connect_changed(move |_| {
                if let Some(n) = w.upgrade() {
                    n.mudou();
                }
            });
        }
        let w = Rc::downgrade(&n);
        mais_var.connect_clicked(move |_| {
            if let Some(n) = w.upgrade() {
                n.variavel("", "");
            }
        });
        let w = Rc::downgrade(&n);
        n.sem_rede.connect_state_set(move |_, on| {
            if let Some(n) = w.upgrade() {
                rede_sub.set_text(if on { tr("no network at all (local addresses blocked too)") } else { tr("it can reach the internet") });
                n.mudou();
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(&n);
        n.pastas.connect_selected_notify(move |d| {
            let Some(n) = w.upgrade() else { return };
            if d.selected() == 3 && !n.preenchendo.get() {
                let w = Rc::downgrade(&n);
                let ps = pastas_sub.clone();
                crate::arquivos::seletor::escolher_pasta(tr("Agents"), tr("Folder the connector can see"), None, move |p| {
                    if let Some(n) = w.upgrade() {
                        ps.set_text(&p.display().to_string());
                        *n.pasta_escolhida.borrow_mut() = Some(p);
                        n.mudou();
                    }
                });
            } else {
                n.mudou();
            }
        });
        let w = Rc::downgrade(&n);
        n.sob_demanda.connect_state_set(move |_, _| {
            if let Some(n) = w.upgrade() {
                n.mudou();
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(&n);
        copiar.connect_clicked(move |b| {
            if let Some(n) = w.upgrade() {
                b.clipboard().set_text(&n.json_texto.text());
            }
        });
        let w = Rc::downgrade(&n);
        cancelar.connect_clicked(move |_| {
            if let Some(n) = w.upgrade() {
                n.voltar(None);
            }
        });
        let w = Rc::downgrade(&n);
        voltar.connect_clicked(move |_| {
            if let Some(n) = w.upgrade() {
                n.voltar(None);
            }
        });
        let w = Rc::downgrade(&n);
        n.adicionar.connect_clicked(move |_| {
            if let Some(n) = w.upgrade() {
                n.salvar();
            }
        });
        n.sem_rede.set_active(false);
        n.sem_rede.set_active(true);
        n.mudou();
        n
    }

    pub fn ao_voltar(&self, f: impl Fn(Option<String>) + 'static) {
        *self.ao_voltar.borrow_mut() = Some(Box::new(f));
    }

    /// A fresh form (the page opens).
    pub fn limpar(self: &Rc<Self>) {
        self.preenchendo.set(true);
        for e in [&self.nome, &self.comando, &self.argumentos, &self.url] {
            e.set_text("");
        }
        while let Some(x) = self.variaveis.first_child() {
            self.variaveis.remove(&x);
        }
        self.vars.borrow_mut().clear();
        self.local.set_active(true);
        self.sem_rede.set_active(true);
        self.pastas.set_selected(0);
        self.pasta_programa.borrow_mut().take();
        self.sob_demanda.set_active(true);
        self.perms.borrow_mut().clear();
        *self.resultado.borrow_mut() = None;
        self.preenchendo.set(false);
        self.abas[0].set_active(true);
        self.mudou();
        if let Some(b) = &self.primeiro_do_catalogo {
            crate::central::posicao_debug("catalogo primeiro", b.upcast_ref());
        }
    }

    fn voltar(&self, adicionado: Option<String>) {
        if let Some(f) = self.ao_voltar.borrow().as_ref() {
            f(adicionado);
        }
    }

    fn variavel(self: &Rc<Self>, nome: &str, valor: &str) {
        let l = caixa(Orientation::Horizontal, 8, "");
        let n = gtk::Entry::new();
        n.add_css_class("rotinas-campo");
        n.set_placeholder_text(Some("DATABASE_URL"));
        n.set_text(nome);
        n.set_width_chars(16);
        l.append(&n);
        let v = gtk::PasswordEntry::new();
        v.add_css_class("rotinas-campo");
        v.set_show_peek_icon(true);
        v.set_text(valor);
        v.set_hexpand(true);
        l.append(&v);
        let chip = rotulo(tr("kept in the vault"), "linha-chip");
        chip.add_css_class("reversivel");
        chip.set_valign(Align::Center);
        l.append(&chip);
        let tirar = gtk::Button::new();
        tirar.add_css_class("memoria-acao");
        tirar.set_child(Some(&ui_icon("lixeira", "#8E8F99", 14)));
        tirar.set_valign(Align::Center);
        l.append(&tirar);
        self.variaveis.append(&l);
        let (w, lw) = (Rc::downgrade(self), l.downgrade());
        tirar.connect_clicked(move |_| {
            if let (Some(s), Some(l)) = (w.upgrade(), lw.upgrade()) {
                s.vars.borrow_mut().retain(|(_, _, x)| x != &l);
                s.variaveis.remove(&l);
                s.mudou();
            }
        });
        for e in [n.upcast_ref::<gtk::Editable>(), v.upcast_ref::<gtk::Editable>()] {
            let w = Rc::downgrade(self);
            e.connect_changed(move |_| {
                if let Some(s) = w.upgrade() {
                    s.mudou();
                }
            });
        }
        self.vars.borrow_mut().push((n, v, l));
        self.mudou();
    }

    fn do_catalogo(self: &Rc<Self>, it: &Value) {
        self.preenchendo.set(true);
        let sistema = conectores_do_sistema().display().to_string();
        let home = glib::home_dir().join("Documentos");
        let subst = |a: &str| a.replace("@KALANGO_CONECTORES@", &sistema).replace("@PASTA@", &home.display().to_string()).replace("@ARQUIVO@", &home.join("banco.db").display().to_string());
        self.nome.set_text(it["id"].as_str().unwrap_or(""));
        *self.pasta_programa.borrow_mut() = it["pasta"].as_str().map(subst);
        match it["url"].as_str() {
            Some(u) => {
                self.remoto.set_active(true);
                self.url.set_text(u);
            }
            None => {
                self.local.set_active(true);
                let c = it["comando"].as_str().unwrap_or("");
                self.comando.set_text(&which(c).unwrap_or_else(|| c.to_string()));
                let args: Vec<String> = it["argumentos"].as_array().into_iter().flatten().filter_map(|a| a.as_str()).map(subst).map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a }).collect();
                self.argumentos.set_text(&args.join(" "));
            }
        }
        while let Some(x) = self.variaveis.first_child() {
            self.variaveis.remove(&x);
        }
        self.vars.borrow_mut().clear();
        self.sem_rede.set_active(!it["rede"].as_bool().unwrap_or(false));
        self.preenchendo.set(false);
        for v in it["ambiente"].as_array().into_iter().flatten().filter_map(|v| v.as_str()) {
            self.variavel(v, "");
        }
        self.abas[2].set_active(true);
        self.mudou();
    }

    fn do_colado(self: &Rc<Self>) {
        let b = self.colar.buffer();
        let texto = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
        let erro = |t: &str| {
            self.colar_erro.set_text(t);
            self.colar_erro.set_visible(true);
        };
        let Ok(v) = serde_json::from_str::<Value>(&texto) else { return erro(tr("This is not valid JSON.")) };
        let servidores = v["mcpServers"].as_object().or(v["servers"].as_object()).or(v.as_object()).cloned().unwrap_or_default();
        let Some((nome, s)) = servidores.into_iter().find(|(_, s)| s["command"].is_string() || s["url"].is_string()) else {
            return erro(tr("No server with a “command” or a “url” in it."));
        };
        self.colar_erro.set_visible(false);
        self.preenchendo.set(true);
        self.nome.set_text(&crate::central_ajustes::nome_valido(&nome));
        match (s["command"].as_str(), s["url"].as_str()) {
            (Some(c), _) => {
                self.local.set_active(true);
                self.comando.set_text(&which(c).unwrap_or_else(|| c.to_string()));
                let args: Vec<String> = s["args"].as_array().into_iter().flatten().filter_map(|a| a.as_str()).map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.to_string() }).collect();
                self.argumentos.set_text(&args.join(" "));
            }
            (None, Some(u)) => {
                self.remoto.set_active(true);
                self.url.set_text(u);
            }
            _ => {}
        }
        while let Some(x) = self.variaveis.first_child() {
            self.variaveis.remove(&x);
        }
        self.vars.borrow_mut().clear();
        self.preenchendo.set(false);
        for (k, val) in s["env"].as_object().into_iter().flatten() {
            self.variavel(k, val.as_str().unwrap_or(""));
        }
        self.abas[2].set_active(true);
        self.mudou();
    }

    /// The arguments as the shell would split them (quotes keep spaces together).
    fn args(&self) -> Vec<String> {
        let mut v = Vec::new();
        let mut atual = String::new();
        let mut aspas = false;
        for c in self.argumentos.text().chars() {
            match c {
                '"' => aspas = !aspas,
                c if c.is_whitespace() && !aspas => {
                    if !atual.is_empty() {
                        v.push(std::mem::take(&mut atual));
                    }
                }
                c => atual.push(c),
            }
        }
        if !atual.is_empty() {
            v.push(atual);
        }
        v
    }

    fn pasta_visivel(&self) -> Option<PathBuf> {
        match self.pastas.selected() {
            1 => Some(crate::pastas::documentos()),
            2 => Some(crate::pastas::downloads()),
            3 => self.pasta_escolhida.borrow().clone(),
            _ => None,
        }
    }

    /// The manifest as it will be written; `segredos`: the variables' values themselves (only for the test, in a
    /// temporary file) or `cofre:NAME` (the real one).
    fn manifesto(&self, segredos: bool) -> Option<toml::Table> {
        let nome = crate::central_ajustes::nome_valido(&self.nome.text());
        if nome.is_empty() {
            return None;
        }
        let mut t = toml::Table::new();
        t.insert("nome".into(), toml::Value::String(nome.clone()));
        if self.local.is_active() {
            let c = self.comando.text().trim().to_string();
            if c.is_empty() {
                return None;
            }
            t.insert("comando".into(), toml::Value::String(which(&c).unwrap_or(c)));
            let args = self.args();
            // the program's own folder: kalangod lets it read (and run) only there and the system
            let pasta = self.pasta_programa.borrow().clone().or_else(|| args.iter().map(std::path::Path::new)
                .find(|a| a.is_absolute() && a.is_file()).and_then(|a| a.parent()).map(|d| d.display().to_string()));
            if let Some(p) = pasta {
                t.insert("pasta".into(), toml::Value::String(p));
            }
            t.insert("argumentos".into(), toml::Value::Array(args.into_iter().map(toml::Value::String).collect()));
            if let Some(p) = self.pasta_visivel() {
                t.insert("leitura".into(), toml::Value::Array(vec![toml::Value::String(p.display().to_string())]));
            }
            t.insert("rede".into(), toml::Value::Boolean(!self.sem_rede.is_active()));
        } else {
            let u = self.url.text().trim().to_string();
            if u.is_empty() {
                return None;
            }
            t.insert("url".into(), toml::Value::String(u));
        }
        let amb: toml::Table = self.vars.borrow().iter().filter(|(k, _, _)| !k.text().trim().is_empty()).map(|(k, v, _)| {
            let k = k.text().trim().to_string();
            let valor = if segredos { v.text().to_string() } else { format!("cofre:{}", nome_no_cofre(&nome, &k)) };
            (k, toml::Value::String(valor))
        }).collect();
        if !amb.is_empty() {
            t.insert("ambiente".into(), toml::Value::Table(amb));
        }
        t.insert("sob_demanda".into(), toml::Value::Boolean(self.sob_demanda.is_active()));
        Some(t)
    }

    /// Something changed: the JSON again, and the test once the typing rests.
    fn mudou(self: &Rc<Self>) {
        if self.preenchendo.get() {
            return;
        }
        let nome = crate::central_ajustes::nome_valido(&self.nome.text());
        let amb: serde_json::Map<String, Value> = self.vars.borrow().iter().filter(|(k, _, _)| !k.text().trim().is_empty())
            .map(|(k, _, _)| (k.text().trim().to_string(), json!(format!("cofre://{}", nome_no_cofre(&nome, k.text().trim()))))).collect();
        let mut s = if self.local.is_active() {
            json!({"command": self.comando.text().trim(), "args": self.args()})
        } else {
            json!({"url": self.url.text().trim()})
        };
        if !amb.is_empty() {
            s["env"] = Value::Object(amb);
        }
        // as the services document it: one key per line, the arguments and the environment on one line each
        let chave = if nome.is_empty() { "conector".to_string() } else { nome.clone() };
        let mut linhas = vec!["{".to_string(), format!("  {}: {{", json!(chave))];
        let campos: Vec<String> = ["command", "url", "args", "env"].iter().filter_map(|k| s.get(*k).map(|v| format!("    {}: {}", json!(k), compacto(v)))).collect();
        let n = campos.len();
        for (i, c) in campos.into_iter().enumerate() {
            linhas.push(if i + 1 < n { format!("{c},") } else { c });
        }
        linhas.push("  }".into());
        linhas.push("}".into());
        let js = linhas.join("\n");
        self.json_texto.set_text(&js.lines().enumerate().map(|(i, l)| format!("{:>2}  {l}", i + 1)).collect::<Vec<_>>().join("\n"));
        // community code: what isolates it, said plainly
        let comunidade = self.local.is_active() && de_comunidade(&self.comando.text());
        if comunidade {
            let rede = if self.sem_rede.is_active() { tr("without the outside network") } else { tr("with network access") };
            let pastas = match self.pasta_visivel() { Some(p) => trf("seeing only {folder}", &[("folder", p.display().to_string())]), None => tr("without seeing your files").to_string() };
            let mut t = trf("It runs isolated: {network} and {folders}.", &[("network", rede.to_string()), ("folders", pastas)]);
            if !self.vars.borrow().is_empty() {
                t.push(' ');
                t.push_str(tr("The values stay in the system vault, out of the configuration file."));
            }
            self.aviso_texto.set_text(&t);
        }
        self.aviso.set_visible(comunidade);
        // the test, 800 ms after the last change
        if let Some(id) = self.espera.borrow_mut().take() {
            id.remove();
        }
        self.adicionar.set_sensitive(false);
        let Some(m) = self.manifesto(true) else {
            self.mostrar_teste(None, false);
            return;
        };
        self.mostrar_teste(None, true);
        let g = self.geracao.get() + 1;
        self.geracao.set(g);
        let w = Rc::downgrade(self);
        *self.espera.borrow_mut() = Some(glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || {
            let Some(n) = w.upgrade() else { return };
            n.espera.borrow_mut().take();
            let w = Rc::downgrade(&n);
            crate::lateral::em_segundo_plano(move || testar_temporario(&m), move |r| {
                let Some(n) = w.upgrade() else { return };
                if n.geracao.get() != g {
                    return;
                }
                *n.resultado.borrow_mut() = Some(r.clone());
                n.mostrar_teste(Some(r), false);
            });
        }));
    }

    fn mostrar_teste(self: &Rc<Self>, r: Option<Result<Value, String>>, testando: bool) {
        for c in [&self.passos, &self.ferramentas] {
            while let Some(x) = c.first_child() {
                c.remove(&x);
            }
        }
        let passo = |ok: Option<bool>, t: &str, ms: Option<u64>| {
            let l = caixa(Orientation::Horizontal, 10, "");
            l.append(&ui_icon(match ok { Some(true) => "check", Some(false) => "fechar", None => "relogio" }, match ok { Some(true) => "#8ED2C3", Some(false) => "#EFA2A6", None => "#8E8F99" }, 14));
            let a = rotulo(t, "central-limite-nome");
            a.set_xalign(0.0);
            a.set_wrap(true);
            a.set_hexpand(true);
            l.append(&a);
            if let Some(ms) = ms {
                l.append(&rotulo(&format!("{:.1} s", ms as f64 / 1000.0).replace('.', ","), "linha-hora"));
            }
            self.passos.append(&l);
        };
        let vazio = |t: &str| {
            let v = rotulo(t, "central-agente-sub");
            v.set_xalign(0.0);
            v.set_wrap(true);
            v.set_margin_top(6);
            v.set_margin_bottom(6);
            self.ferramentas.append(&v);
        };
        match r {
            None if testando => {
                passo(None, tr("testing: starting it isolated…"), None);
                vazio(tr("The tools show up after the test."));
            }
            None => {
                passo(None, tr("Fill in a name and a command or an address."), None);
                vazio(tr("Nothing is added before the test."));
            }
            Some(Err(e)) => {
                let base = self.comando.text().to_string();
                let (texto, _) = crate::central_ajustes::diagnostico(&e, &e, &base, !self.sem_rede.is_active(), !self.local.is_active());
                passo(Some(false), &texto, None);
                vazio(tr("It is added only after it answers."));
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug central: novo conector falhou ({e})");
                }
            }
            Some(Ok(v)) => {
                let tempos: BTreeMap<String, u64> = v["tempos_ms"].as_array().into_iter().flatten().filter_map(|t| Some((t["etapa"].as_str()?.to_string(), t["ms"].as_u64()?))).collect();
                passo(Some(true), tr("process started isolated"), tempos.get("processo").copied());
                passo(Some(true), tr("MCP handshake done"), tempos.get("initialize").copied());
                let fs = v["ferramentas"].as_array().cloned().unwrap_or_default();
                passo(Some(true), &if fs.len() == 1 { tr("1 tool found").to_string() } else { trf("{n} tools found", &[("n", fs.len().to_string())]) }, tempos.get("tools/list").copied());
                for f in &fs {
                    let nome = f["nome"].as_str().unwrap_or("").to_string();
                    let leitura = f["leitura"] == true;
                    let destrutiva = f["destrutiva"] == true;
                    let l = caixa(Orientation::Horizontal, 8, "ajustes-ferramenta");
                    let c = caixa(Orientation::Vertical, 1, "");
                    c.set_hexpand(true);
                    let a = rotulo(&nome, "ajustes-ferramenta-nome");
                    a.set_xalign(0.0);
                    c.append(&a);
                    let d = rotulo(f["descricao"].as_str().unwrap_or(""), "central-agente-sub");
                    d.set_xalign(0.0);
                    d.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    d.set_max_width_chars(1);
                    d.set_hexpand(true);
                    c.append(&d);
                    l.append(&c);
                    let tipo = rotulo(if destrutiva { tr("destructive (kind)") } else if leitura { tr("reading (kind)") } else { tr("writes (kind)") }, "linha-chip");
                    tipo.add_css_class(if destrutiva { "negado" } else if leitura { "leitura" } else { "espera" });
                    tipo.set_valign(Align::Center);
                    l.append(&tipo);
                    // the default (read only: free; destructive: never; else ask), or what the person set here
                    let padrao = if leitura { "livre" } else if destrutiva { "nunca" } else { "perguntar" };
                    let p = self.perms.borrow().get(&nome).cloned().unwrap_or_else(|| padrao.to_string());
                    let pb = gtk::Button::with_label(match p.as_str() { "livre" => tr("free"), "nunca" => tr("never"), _ => tr("ask") });
                    pb.add_css_class("central-celula");
                    pb.add_css_class(&p);
                    pb.set_valign(Align::Center);
                    pb.set_tooltip_text(Some(tr("Click to change")));
                    let w = Rc::downgrade(self);
                    pb.connect_clicked(move |_| {
                        if let Some(n) = w.upgrade() {
                            let prox = match p.as_str() { "livre" => "perguntar", "perguntar" => "nunca", _ => "livre" };
                            n.perms.borrow_mut().insert(nome.clone(), prox.into());
                            let r = n.resultado.borrow().clone();
                            n.mostrar_teste(r, false);
                        }
                    });
                    l.append(&pb);
                    self.ferramentas.append(&l);
                }
                if fs.is_empty() {
                    vazio(tr("It offers no tools."));
                }
                self.adicionar.set_sensitive(true);
                crate::central::posicao_debug("adicionar conector", self.adicionar.upcast_ref());
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug central: novo conector testado ({} ferramentas)", fs.len());
                }
            }
        }
    }

    /// Adds it: the values to the vault, the manifest, the permissions set here; then back to the list.
    fn salvar(self: &Rc<Self>) {
        let Some(m) = self.manifesto(false) else { return };
        let nome = m["nome"].as_str().unwrap_or("").to_string();
        let segredos: Vec<(String, String)> = self.vars.borrow().iter().filter(|(k, v, _)| !k.text().trim().is_empty() && !v.text().is_empty())
            .map(|(k, v, _)| (nome_no_cofre(&nome, k.text().trim()), v.text().to_string())).collect();
        let perms: Vec<(String, String)> = self.perms.borrow().iter().map(|(t, p)| (format!("{nome}.{t}"), p.clone())).collect();
        let padroes: Vec<(String, String)> = match self.resultado.borrow().as_ref() {
            Some(Ok(v)) => v["ferramentas"].as_array().into_iter().flatten().filter(|f| f["destrutiva"] == true && !self.perms.borrow().contains_key(f["nome"].as_str().unwrap_or("")))
                .map(|f| (format!("{nome}.{}", f["nome"].as_str().unwrap_or("")), "nunca".to_string())).collect(),
            _ => vec![],
        };
        self.adicionar.set_sensitive(false);
        let w = Rc::downgrade(self);
        let bin = crate::voz::agent_binary();
        crate::lateral::em_segundo_plano(move || -> Result<(), String> {
            for (k, v) in &segredos {
                let mut c = Command::new(crate::central_ajustes::kalangod_bin()).args(["cofre", "guardar", k]).stdin(Stdio::piped()).stdout(Stdio::null()).spawn().map_err(|e| e.to_string())?;
                c.stdin.take().ok_or("stdin")?.write_all(format!("{v}\n").as_bytes()).map_err(|e| e.to_string())?;
                if !c.wait().map(|s| s.success()).unwrap_or(false) {
                    return Err(trf("the vault did not keep {name}", &[("name", k.clone())]));
                }
            }
            let dir = crate::central_ajustes::pasta();
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(dir.join(format!("{nome}.toml")), toml::to_string(&m).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            // destructive tools start at "never"; the rest as set here (or their defaults)
            for (t, p) in padroes.iter().chain(&perms) {
                let _ = Command::new(&bin).args(["permissao", t, p]).status();
            }
            Ok(())
        }, move |r| {
            let Some(n) = w.upgrade() else { return };
            match r {
                Ok(()) => {
                    let nome = n.manifesto(false).and_then(|m| m["nome"].as_str().map(str::to_string));
                    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                        eprintln!("debug central: conector adicionado {}", nome.clone().unwrap_or_default());
                    }
                    n.voltar(nome);
                }
                Err(e) => {
                    n.adicionar.set_sensitive(true);
                    n.mostrar_teste(Some(Err(e)), false);
                }
            }
        });
    }
}

/// The handshake on a temporary manifest (the values in it, for the test only; the file goes right after).
fn testar_temporario(m: &toml::Table) -> Result<Value, String> {
    let dir = std::env::temp_dir().join(format!("kalango-conector-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    let p = dir.join(format!("{}.toml", m["nome"].as_str().unwrap_or("x")));
    std::fs::write(&p, toml::to_string(m).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let o = Command::new(crate::central_ajustes::kalangod_bin()).args(["conector", "testar"]).arg(&p).output();
    let _ = std::fs::remove_file(&p);
    let o = o.map_err(|e| e.to_string())?;
    if !o.status.success() {
        let e = String::from_utf8_lossy(&o.stderr).trim().trim_start_matches("kalangod: ").to_string();
        return Err(if e.is_empty() { tr("the connector did not answer").to_string() } else { e });
    }
    serde_json::from_slice(&o.stdout).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_json_keeps_strings_whole() {
        let v = json!({"url": "https://x.example/mcp", "args": ["-y", "a,b"], "env": {"K": "cofre://x.k"}});
        assert_eq!(compacto(&v["url"]), "\"https://x.example/mcp\"");
        assert_eq!(compacto(&v["args"]), "[\"-y\", \"a,b\"]");
        assert_eq!(compacto(&v["env"]), "{ \"K\": \"cofre://x.k\" }");
        assert_eq!(nome_no_cofre("postgres-local", "DATABASE_URL"), "postgres-local.database-url");
        assert!(de_comunidade("/usr/bin/npx") && !de_comunidade("/usr/bin/python3"));
    }
}
