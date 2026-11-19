//! Central de agentes (design "Central de agentes"): a window of the system (app id os.kalango.Agentes, so the dock
//! and the window switcher know it). "Em execução": how many agents work and how many wait for the person, the CPU,
//! the GPU and today's tokens, a card per task — the agent of the side bar while it works (pause, stop), each
//! request waiting in kalangod (see it, refuse it), and the tasks that ended (done or failed, with why) — and, on the
//! right, the chosen task's steps with their times. "Pedir uma nova tarefa…" gives the agent a request.
//!
//! The rail lists every page of the center; the ones this build does not have yet are shown, but cannot be opened.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, estrela, rotulo};
use crate::widgets::comum::{barra, Suave};
use crate::widgets::dados::{self, Amostra};
use crate::Shell;
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{n_, tr, trf};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

/// The rail: (page, name, glyph, available in this build).
pub(crate) const PAGINAS: &[(&str, &str, &str, bool)] = &[
    ("conversas", n_("Conversations"), "enviar", true),
    ("agentes", n_("Agents"), "spark", true),
    ("rotinas", n_("Routines"), "raio", true),
    ("linha", n_("Timeline"), "relogio", true),
    ("memoria", n_("Memory"), "cerebro", true),
    ("permissoes", n_("Permissions"), "escudo", true),
    ("uso", n_("Usage"), "cpu", true),
    ("ajustes", n_("Agent settings"), "ajustes", true),
];

extern "C" {
    fn gdk_wayland_toplevel_set_application_id(toplevel: *mut gtk::gdk::ffi::GdkToplevel, id: *const std::ffi::c_char);
}

/// A task card's data.
#[derive(Clone, Debug, PartialEq)]
enum Tarefa {
    Rodando { pedido: String, passos: usize, inicio: i64, tokens: u64, pausado: bool },
    /// a routine running now (alone)
    Rotina { id: u64, nome: String, inicio: i64 },
    Esperando { id: u64, quem: String, pedido: String, desde: i64, prazo: u64 },
    Fim { pedido: String, estado: String, motivo: String, fim: i64, tokens: u64, passos: Vec<(String, String, i64)>, rotina: bool },
}

fn hora(t: i64) -> String {
    glib::DateTime::from_unix_local(t).ok().and_then(|d| d.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default()
}

fn hora_s(t: i64) -> String {
    glib::DateTime::from_unix_local(t).ok().and_then(|d| d.format("%H:%M:%S").ok()).map(|s| s.to_string()).unwrap_or_default()
}

fn mil(n: u64) -> String {
    if n >= 1000 { format!("{}k", (n + 500) / 1000) } else { n.to_string() }
}

/// For the session test: where a control is in its window (once it is laid out).
pub(crate) fn posicao_debug(nome: &str, w: &gtk::Widget) {
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_none() {
        return;
    }
    // not on screen yet (a hidden tab's page, a window still being built): its place is measured once it shows (a hidden
    // widget's bounds are not where a click would land)
    if !w.is_mapped() {
        let nome = nome.to_string();
        let id: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::default();
        let id2 = id.clone();
        *id.borrow_mut() = Some(w.connect_map(move |w| {
            if let Some(h) = id2.borrow_mut().take() {
                w.disconnect(h);
            }
            posicao_debug(&nome, w);
        }));
        return;
    }
    medir_depois(nome, w, 0);
}

/// Like `posicao_debug`, but again every time the widget shows (a window shown again, a tab opened): its place can
/// change from one showing to the next (the session test reads the latest).
pub(crate) fn posicao_debug_sempre(nome: &str, w: &gtk::Widget) {
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_none() {
        return;
    }
    let n = nome.to_string();
    w.connect_map(move |w| medir_depois(&n, w, 0));
    if w.is_mapped() {
        medir_depois(nome, w, 0);
    }
}

fn medir_depois(nome: &str, w: &gtk::Widget, tentativa: u32) {
    let (nome, w) = (nome.to_string(), w.downgrade());
    glib::timeout_add_local_once(Duration::from_millis(400), move || {
        let Some(w) = w.upgrade() else { return };
        let Some(raiz) = w.root() else { return };
        // inside a popover (its own surface): its place in the popover, plus where the popover is on the window
        if let Some(pop) = w.native().and_downcast::<gtk::Popover>() {
            let (Some(b), Some(popup)) = (w.compute_bounds(&pop), pop.surface().and_then(|s| s.dynamic_cast::<gtk::gdk::Popup>().ok())) else { return };
            let (px, py) = (popup.position_x() as f32, popup.position_y() as f32);
            let (ptx, pty) = pop.surface_transform();
            let (jtx, jty) = raiz.surface_transform();
            eprintln!("debug central: {nome} em {:.0} {:.0}", px + ptx as f32 - jtx as f32 + b.x() + b.width() / 2.0, py + pty as f32 - jty as f32 + b.y() + b.height() / 2.0);
            return;
        }
        if let Some(b) = w.compute_bounds(&raiz) {
            // a window still being built (its first size not given yet): measured again a little later
            if (b.width() < 1.0 || b.y() < 0.0) && tentativa < 10 {
                medir_depois(&nome, &w, tentativa + 1);
                return;
            }
            eprintln!("debug central: {nome} em {:.0} {:.0}", b.x() + b.width() / 2.0, b.y() + b.height() / 2.0);
        }
    });
}

pub struct Central {
    janela: gtk::ApplicationWindow,
    paginas: gtk::Stack,
    trilho: Vec<(&'static str, gtk::Button)>,
    titulo: gtk::Label,
    pedido: gtk::Entry,
    permissoes: Rc<crate::central_permissoes::Permissoes>,
    memoria: Rc<crate::central_memoria::Memoria>,
    rotinas: Rc<crate::central_rotinas::PaginaRotinas>,
    linha_tempo: Rc<crate::central_linha::Linha>,
    uso: Rc<crate::central_uso::PaginaUso>,
    ajustes: Rc<crate::central_ajustes::Ajustes>,
    conversas: Rc<crate::central_conversas::Conversas>,
    /// "Criar agente (por descrição)": reached from "+ criar agente"; the rail shows Agents
    novo_agente: Rc<crate::new_agent::NewAgentPage>,
    /// "Instalar agente": a third-party agent's manifest, its permissions adjusted before installing
    instalar_agente: Rc<crate::install_agent::InstallAgentPage>,
    /// the rail, the toolbar and its line (the conversations page has its own side bar and headers)
    trilho_box: gtk::Box,
    alca: gtk::WindowHandle,
    linha_sep: gtk::Box,
    tique: Cell<u32>,
    atual: RefCell<&'static str>,
    shell: RefCell<Weak<Shell>>,
    subtitulo: gtk::Label,
    cartoes: gtk::Grid,
    linha_titulo: gtk::Label,
    linha: gtk::Box,
    escolhida: Cell<usize>,
    pendentes: RefCell<Vec<Value>>,
    assinatura: RefCell<String>,
}

impl Central {
    pub fn new(app: &gtk::Application, shell: &Rc<Shell>) -> Rc<Central> {
        let janela = gtk::ApplicationWindow::new(app);
        janela.set_title(Some(tr("Agents")));
        // (for the session test: where the keyboard's focus goes — a form's field, the search, a button)
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            janela.connect_focus_widget_notify(|j| {
                if let Some(w) = gtk::prelude::GtkWindowExt::focus(j) {
                    let nome = w.downcast_ref::<gtk::Button>().and_then(|b| b.label()).map(|l| l.to_string())
                        .or_else(|| w.css_classes().first().map(|c| c.to_string()))
                        .unwrap_or_else(|| w.type_().name().to_string());
                    eprintln!("debug central: foco em {nome} ({})", w.type_().name());
                }
            });
        }
        // the design's 1520 × 900, or less on a smaller screen (room for the bar and the dock)
        let (mw, mh) = gtk::gdk::Display::default().and_then(|d| d.monitors().item(0)).and_downcast::<gtk::gdk::Monitor>()
            .map(|m| (m.geometry().width(), m.geometry().height())).unwrap_or((1920, 1080));
        let largura = 1520.min(mw - 80);
        janela.set_default_size(largura, 900.min(mh - 140));
        janela.add_css_class("central");
        janela.set_hide_on_close(true);
        // the window draws its own bar (toolbar with the controls): no default title bar
        // (hidden: the theme gives a title bar a minimum height, even an empty one)
        let sem_barra = gtk::Box::new(Orientation::Horizontal, 0);
        sem_barra.set_visible(false);
        janela.set_titlebar(Some(&sem_barra));
        let raiz = caixa(Orientation::Horizontal, 0, "");
        // the rail
        let trilho = caixa(Orientation::Vertical, 4, "central-trilho");
        trilho.set_size_request(64, -1);
        let logo = crate::apps::imagem_kalango("agentes", 36);
        logo.set_margin_top(12);
        logo.set_margin_bottom(6);
        trilho.append(&logo);
        let div = caixa(Orientation::Vertical, 0, "central-trilho-div");
        div.set_size_request(28, 1);
        div.set_halign(Align::Center);
        div.set_margin_bottom(4);
        trilho.append(&div);
        let mut botoes_trilho = Vec::new();
        for (id, nome, _, pronta) in PAGINAS {
            let b = gtk::Button::new();
            b.add_css_class("central-trilho-item");
            b.set_tooltip_text(Some(&if *pronta { tr(nome).to_string() } else { trf("{page} (not in this version yet)", &[("page", tr(nome).to_string())]) }));
            b.set_sensitive(*pronta);
            b.set_halign(Align::Center);
            trilho.append(&b);
            botoes_trilho.push((*id, b));
        }
        // (on the conversations' page, below: the button that opens their side bar, and the person)
        let te = caixa(Orientation::Vertical, 0, "");
        te.set_vexpand(true);
        trilho.append(&te);
        let trilho_lista = gtk::Button::new();
        trilho_lista.add_css_class("central-trilho-item");
        trilho_lista.set_child(Some(&ui_icon("painel-esquerda", "#8E8F99", 16)));
        trilho_lista.set_tooltip_text(Some(tr("Open the side bar")));
        trilho_lista.set_halign(Align::Center);
        trilho_lista.set_visible(false);
        trilho.append(&trilho_lista);
        let pessoa = glib::real_name().to_string_lossy().split(',').next().unwrap_or("").trim().to_string();
        let pessoa = if pessoa.is_empty() || pessoa == "Unknown" { glib::user_name().to_string_lossy().into_owned() } else { pessoa };
        let trilho_pessoa = caixa(Orientation::Horizontal, 0, "conversas-avatar");
        trilho_pessoa.set_size_request(28, 28);
        trilho_pessoa.set_halign(Align::Center);
        trilho_pessoa.set_margin_top(4);
        trilho_pessoa.set_margin_bottom(12);
        trilho_pessoa.set_tooltip_text(Some(&pessoa));
        let ini = rotulo(&pessoa.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default(), "conversas-avatar-letra");
        ini.set_hexpand(true);
        trilho_pessoa.append(&ini);
        trilho_pessoa.set_visible(false);
        // (its letter fills the circle: the rail itself keeps its 64 px)
        trilho_pessoa.set_hexpand(false);
        trilho.append(&trilho_pessoa);
        trilho.set_hexpand(false);
        posicao_debug_sempre("central trilho lista", trilho_lista.upcast_ref());
        raiz.append(&trilho);
        // the toolbar
        let principal = caixa(Orientation::Vertical, 0, "");
        principal.set_hexpand(true);
        let barra_f = caixa(Orientation::Horizontal, 10, "central-barra");
        let titulo = rotulo(tr("Agents"), "central-titulo-janela");
        barra_f.append(&titulo);
        let espaco = caixa(Orientation::Horizontal, 0, "");
        espaco.set_hexpand(true);
        barra_f.append(&espaco);
        let busca = caixa(Orientation::Horizontal, 7, "central-busca");
        busca.set_size_request(300, 32);
        busca.set_hexpand(false);
        busca.append(&estrela(13, "#A5A1E2"));
        let pedido = gtk::Entry::new();
        pedido.set_hexpand(true);
        pedido.add_css_class("central-busca-campo");
        pedido.set_placeholder_text(Some(tr("Ask for a new task…")));
        busca.append(&pedido);
        let atalho = rotulo("Ctrl K", "visao-atalho");
        atalho.set_valign(Align::Center);
        busca.append(&atalho);
        busca.set_valign(Align::Center);
        barra_f.append(&busca);
        let menu = gtk::MenuButton::new();
        menu.add_css_class("central-menu");
        menu.set_child(Some(&ui_icon("menu", "#8E8F99", 16)));
        let pop = gtk::Popover::new();
        let pc = caixa(Orientation::Vertical, 2, "");
        let lateral = gtk::Button::with_label(tr("Open the agent bar"));
        lateral.add_css_class("central-menu-item");
        pc.append(&lateral);
        pop.set_child(Some(&pc));
        menu.set_popover(Some(&pop));
        menu.set_valign(Align::Center);
        barra_f.append(&menu);
        let controles = caixa(Orientation::Horizontal, 8, "");
        controles.set_valign(Align::Center);
        let j = janela.downgrade();
        for (glifo, acao) in [("minimizar", 0), ("maximizar", 1), ("fechar", 2)] {
            let b = gtk::Button::new();
            b.add_css_class("central-controle");
            b.set_child(Some(&ui_icon(glifo, "#8E8F99", 16)));
            let j = j.clone();
            b.connect_clicked(move |_| {
                let Some(j) = j.upgrade() else { return };
                match acao {
                    0 => j.minimize(),
                    1 => if j.is_maximized() { j.unmaximize() } else { j.maximize() },
                    _ => j.close(),
                }
            });
            controles.append(&b);
        }
        barra_f.append(&controles);
        let alca = gtk::WindowHandle::new();
        alca.set_child(Some(&barra_f));
        principal.append(&alca);
        let linha_sep = caixa(Orientation::Horizontal, 0, "central-linha");
        principal.append(&linha_sep);
        // the pages; "Em execução": body and the steps on the right
        let paginas = gtk::Stack::new();
        paginas.set_vexpand(true);
        // (each page's own minimum width: the widest page does not hold the window wide on the others)
        paginas.set_hhomogeneous(false);
        paginas.set_transition_type(gtk::StackTransitionType::Crossfade);
        paginas.set_transition_duration(140);
        let conteudo = caixa(Orientation::Horizontal, 0, "");
        conteudo.set_vexpand(true);
        let corpo = caixa(Orientation::Vertical, 18, "central-corpo");
        corpo.set_hexpand(true);
        let t = caixa(Orientation::Vertical, 2, "");
        let h = rotulo(tr("In progress"), "central-titulo");
        h.set_xalign(0.0);
        t.append(&h);
        let subtitulo = rotulo("", "desk-metrica-sub");
        subtitulo.set_xalign(0.0);
        t.append(&subtitulo);
        corpo.append(&t);
        let recursos = caixa(Orientation::Horizontal, 12, "");
        recursos.set_homogeneous(true);
        let recurso = |nome: &str, cor: &str| {
            let c = caixa(Orientation::Vertical, 8, "central-recurso");
            let l = caixa(Orientation::Horizontal, 0, "");
            let n = rotulo(nome, "desk-metrica-sub");
            n.set_hexpand(true);
            n.set_xalign(0.0);
            l.append(&n);
            let v = rotulo("—", "desk-metrica-nome");
            l.append(&v);
            c.append(&l);
            let (b, s): (gtk::DrawingArea, Suave) = barra(200, cor);
            b.set_hexpand(true);
            b.set_size_request(-1, 3);
            c.append(&b);
            recursos.append(&c);
            (v, b, s)
        };
        let (cpu_v, cpu_b, cpu_s) = recurso(tr("CPU"), "#8ED2C3");
        let (gpu_v, gpu_b, gpu_s) = recurso(tr("GPU"), "#A5A1E2");
        let (tok_v, tok_b, tok_s) = recurso(tr("Tokens today"), "#E5C98C");
        corpo.append(&recursos);
        let cartoes = gtk::Grid::new();
        cartoes.set_valign(Align::Start);
        cartoes.set_column_spacing(14);
        cartoes.set_row_spacing(14);
        cartoes.set_column_homogeneous(true);
        let rolagem = gtk::ScrolledWindow::new();
        rolagem.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        rolagem.set_vexpand(true);
        rolagem.set_child(Some(&cartoes));
        corpo.append(&rolagem);
        conteudo.append(&corpo);
        let painel = caixa(Orientation::Vertical, 14, "central-passos");
        painel.set_size_request(380, -1);
        let cab = caixa(Orientation::Horizontal, 6, "");
        cab.append(&ui_icon("menu", "#8E8F99", 13));
        let linha_titulo = rotulo("", "desk-rotulo");
        linha_titulo.set_ellipsize(gtk::pango::EllipsizeMode::End);
        cab.append(&linha_titulo);
        painel.append(&cab);
        let linha = caixa(Orientation::Vertical, 6, "");
        painel.append(&linha);
        let logs = gtk::Button::with_label(tr("Full logs ›"));
        logs.add_css_class("desk-link");
        logs.set_halign(Align::Start);
        painel.append(&logs);
        conteudo.append(&painel);
        paginas.add_named(&conteudo, Some("agentes"));
        let permissoes = crate::central_permissoes::Permissoes::new(shell, largura);
        paginas.add_named(&permissoes.raiz, Some("permissoes"));
        let memoria = crate::central_memoria::Memoria::new(shell, largura);
        paginas.add_named(&memoria.raiz, Some("memoria"));
        let rotinas = crate::central_rotinas::PaginaRotinas::new(shell, largura);
        paginas.add_named(&rotinas.raiz, Some("rotinas"));
        let linha_tempo = crate::central_linha::Linha::new(shell, largura);
        paginas.add_named(&linha_tempo.raiz, Some("linha"));
        let uso = crate::central_uso::PaginaUso::new(shell, largura);
        paginas.add_named(&uso.raiz, Some("uso"));
        let ajustes = crate::central_ajustes::Ajustes::new(shell, largura);
        paginas.add_named(&ajustes.raiz, Some("ajustes"));
        let conversas = crate::central_conversas::Conversas::new(shell, largura);
        paginas.add_named(&conversas.raiz, Some("conversas"));
        let novo_agente = crate::new_agent::NewAgentPage::new();
        paginas.add_named(&novo_agente.raiz, Some("novo-agente"));
        let instalar_agente = crate::install_agent::InstallAgentPage::new();
        paginas.add_named(&instalar_agente.raiz, Some("instalar-agente"));
        conversas.lateral.set_visible(false);
        principal.append(&paginas);
        raiz.append(&conversas.lateral);
        raiz.append(&principal);
        // (over the window: the conversations' sheets in a narrow window, and what measures its width)
        let sobre = gtk::Overlay::new();
        sobre.set_child(Some(&raiz));
        let medidor = gtk::DrawingArea::new();
        medidor.set_can_target(false);
        medidor.set_hexpand(true);
        medidor.set_vexpand(true);
        sobre.add_overlay(&medidor);
        sobre.set_measure_overlay(&medidor, false);
        janela.set_child(Some(&sobre));

        let c = Rc::new(Central {
            janela, paginas, trilho: botoes_trilho, titulo, pedido: pedido.clone(), permissoes, memoria, rotinas, linha_tempo, uso, ajustes, conversas, novo_agente, instalar_agente, trilho_box: trilho.clone(), alca: alca.clone(), linha_sep, tique: Cell::new(0), atual: RefCell::new("agentes"), shell: RefCell::new(Rc::downgrade(shell)), subtitulo, cartoes, linha_titulo, linha,
            escolhida: Cell::new(0), pendentes: RefCell::default(), assinatura: RefCell::default(),
        });
        // the window carries the Agents app id (the dock, Alt+Tab and the overview show it as that app)
        // (at realize GtkApplication sets its own; at map the toplevel exists and the id goes straight to the
        // compositor)
        let app_id = |j: &gtk::ApplicationWindow| {
            if let Some(s) = j.surface().filter(|s| s.type_().name().contains("Wayland")) {
                let id = std::ffi::CString::new("os.kalango.Agentes").unwrap();
                unsafe {
                    gdk_wayland_toplevel_set_application_id(s.as_ptr() as *mut gtk::gdk::ffi::GdkToplevel, id.as_ptr());
                }
            }
        };
        c.janela.connect_realize(move |j| app_id(j));
        c.janela.connect_map(move |j| app_id(j));
        for (id, b) in &c.trilho {
            let (w, id) = (Rc::downgrade(&c), *id);
            b.connect_clicked(move |_| {
                if let Some(c) = w.upgrade() {
                    c.pagina(id);
                }
            });
        }
        // the conversations' side bar: its pages go through here; collapsing it shows the rail
        let w = Rc::downgrade(&c);
        c.conversas.ao_ir(move |id| {
            if let Some(c) = w.upgrade() {
                c.pagina(id);
            }
        });
        let w = Rc::downgrade(&c);
        *c.novo_agente.ir.borrow_mut() = Some(Box::new(move |id| {
            if let Some(c) = w.upgrade() {
                c.pagina(id);
            }
        }));
        let w = Rc::downgrade(&c);
        *c.instalar_agente.ir.borrow_mut() = Some(Box::new(move |id| {
            if let Some(c) = w.upgrade() {
                c.pagina(id);
            }
        }));
        let w = Rc::downgrade(&c);
        let (tl, tp) = (trilho_lista.clone(), trilho_pessoa.clone());
        c.conversas.ao_recolher(move |trilho, botao| {
            if let Some(c) = w.upgrade() {
                c.trilho_box.set_visible(trilho);
            }
            tl.set_visible(botao);
            tp.set_visible(botao);
        });
        let w = Rc::downgrade(&c);
        trilho_lista.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                c.conversas.abrir_lista(true);
            }
        });
        // the window's width: the conversations' side bars follow its band
        c.conversas.set_folhas(&sobre);
        let w = Rc::downgrade(&c);
        medidor.connect_resize(move |_, largura, _| {
            if let Some(c) = w.upgrade() {
                glib::idle_add_local_once(move || c.conversas.largura_janela(largura));
            }
        });
        c.pagina("agentes");
        let w = Rc::downgrade(&c);
        pedido.connect_changed(move |e| {
            if let Some(c) = w.upgrade() {
                match *c.atual.borrow() {
                    "memoria" => c.memoria.filtrar(&e.text()),
                    "linha" => c.linha_tempo.filtrar(&e.text()),
                    _ => {}
                }
            }
        });
        let w = Rc::downgrade(&c);
        pedido.connect_activate(move |e| {
            let t = e.text().trim().to_string();
            if t.is_empty() {
                return;
            }
            // on the memory page the field searches; "esqueça…", "lembre…" go to the agent
            if w.upgrade().is_some_and(|c| *c.atual.borrow() == "memoria") && !crate::central_memoria::e_pedido(&t) {
                return;
            }
            // the timeline's field only searches
            if w.upgrade().is_some_and(|c| *c.atual.borrow() == "linha") {
                return;
            }
            e.set_text("");
            if let Some(s) = w.upgrade().and_then(|c| c.shell.borrow().upgrade()) {
                s.right_bar().ask_agent(&t);
            }
        });
        // "Logs completos ›": the timeline
        let w = Rc::downgrade(&c);
        logs.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                c.pagina("linha");
            }
        });
        let w = Rc::downgrade(&c);
        lateral.connect_clicked(move |_| {
            if let Some(s) = w.upgrade().and_then(|c| c.shell.borrow().upgrade()) {
                s.right_bar().abrir(crate::lateral::Modo::Agente);
            }
        });
        // resources
        dados::ouvir(&c.cartoes, move |a: &Amostra| {
            cpu_v.set_text(&format!("{:.0}%", a.cpu * 100.0));
            cpu_s.ir(a.cpu, &cpu_b);
            match &a.gpu {
                Some(g) => {
                    gpu_v.set_text(&format!("{:.0}%", g.uso * 100.0));
                    gpu_s.ir(g.uso, &gpu_b);
                }
                None => gpu_v.set_text("—"),
            }
        });
        let uso: Rc<RefCell<crate::widgets::ia::Uso>> = Rc::default();
        let w = Rc::downgrade(&c);
        let atualizar = move || {
            let Some(c) = w.upgrade() else { return false };
            if c.janela.is_visible() {
                uso.borrow_mut().atualizar();
                let u = uso.borrow();
                let orc = crate::widgets::ia::orcamento();
                tok_v.set_text(&format!("{} / {}", mil(u.local + u.nuvem), mil(orc)));
                tok_s.ir((u.local + u.nuvem) as f64 / orc as f64, &tok_b);
                c.render();
                // the memory page follows the agent: a memory remembered or forgotten shows at once
                if *c.atual.borrow() == "memoria" {
                    c.memoria.verificar();
                }
                // the timeline follows the agents (every 5 s)
                c.tique.set(c.tique.get() + 1);
                if *c.atual.borrow() == "linha" && c.tique.get() % 5 == 0 {
                    c.linha_tempo.recarregar();
                }
                if *c.atual.borrow() == "uso" && c.tique.get() % 10 == 0 {
                    c.uso.render();
                }
                // the conversation's context follows the agent
                if *c.atual.borrow() == "conversas" {
                    c.conversas.atualizar(false);
                }
            }
            true
        };
        atualizar();
        glib::timeout_add_local(Duration::from_secs(1), move || if atualizar() { glib::ControlFlow::Continue } else { glib::ControlFlow::Break });
        // the requests waiting, every 2 s (in the background)
        let w = Rc::downgrade(&c);
        glib::timeout_add_local(Duration::from_secs(2), move || {
            let Some(c) = w.upgrade() else { return glib::ControlFlow::Break };
            if c.janela.is_visible() {
                let w = Rc::downgrade(&c);
                crate::lateral::em_segundo_plano(|| crate::servicos::kalangod_pendentes().unwrap_or_default(), move |p| {
                    if let Some(c) = w.upgrade() {
                        *c.pendentes.borrow_mut() = p;
                    }
                });
            }
            glib::ControlFlow::Continue
        });
        c
    }

    /// A new conversation with this request (`kalango-shell conversar TEXTO`).
    pub fn conversas(&self) -> Rc<crate::central_conversas::Conversas> {
        self.conversas.clone()
    }

    pub fn conversar(self: &Rc<Self>, pedido: &str) {
        self.conversar_com(pedido, Vec::new());
    }

    /// A new conversation with this request and these files.
    pub fn conversar_com(self: &Rc<Self>, pedido: &str, anexos: Vec<std::path::PathBuf>) {
        self.pagina("conversas");
        self.conversas.nova();
        for a in anexos {
            self.conversas.anexar(a);
        }
        self.conversas.comecar(pedido);
    }

    /// Shows a page of the rail (the toolbar's title and field follow it).
    /// "Instalar agente" for a manifest (`kalango-shell instalar-agente FILE`, or opening one in Files).
    pub fn instalar_agente(self: &Rc<Self>, arquivo: &std::path::Path) {
        self.instalar_agente.open(arquivo);
        self.pagina("instalar-agente");
    }

    pub fn pagina(self: &Rc<Self>, id: &str) {
        // (the new agent's page belongs to Agents on the rail)
        let novo = id == "novo-agente";
        let id_trilho = if novo || id == "instalar-agente" { "agentes" } else { id };
        let Some((pid, nome, _, true)) = PAGINAS.iter().find(|p| p.0 == id_trilho) else { return };
        // ("again" only while the window is on screen: a closed window reopened on this page starts as its band wants
        // — suite run 53, §80 found the list opened as if the rail's button had been pressed)
        let ja_aqui = self.janela.is_visible() && *self.atual.borrow() == *pid;
        *self.atual.borrow_mut() = pid;
        self.pedido.set_text("");
        for ((pid, _, glifo, _), (_, b)) in PAGINAS.iter().zip(&self.trilho) {
            let ativa = *pid == id_trilho;
            if ativa { b.add_css_class("ativo") } else { b.remove_css_class("ativo") }
            let cor = if ativa { "#A5A1E2" } else { "#8E8F99" };
            // (the star fills its whole box; the other glyphs have a margin inside theirs)
            b.set_child(Some(&if *glifo == "spark" { estrela(12, cor).upcast::<gtk::Widget>() } else { ui_icon(glifo, cor, 16).upcast() }));
        }
        self.paginas.set_visible_child_name(id);
        // the conversations: the rail opens into their side bar (again, from the rail's own button) and the page
        // brings its own header rows
        let conversas = id == "conversas";
        self.alca.set_visible(!conversas);
        self.linha_sep.set_visible(!conversas);
        // (the side bar as the window's band wants it; the rail's own button, on this page already, opens it)
        self.conversas.na_pagina(conversas, conversas && ja_aqui);
        self.titulo.set_text(if novo { tr("New agent") } else if id == "instalar-agente" { tr("Install agent") } else if id == "agentes" { tr("Agents") } else { tr(nome) });
        if novo {
            self.novo_agente.reset();
        }
        self.pedido.set_placeholder_text(Some(match id {
            "agentes" => tr("Ask for a new task…"),
            "memoria" => tr("Search the memory or ask: “forget…”"),
            "linha" => tr("Search the timeline…"),
            "ajustes" => tr("Search or ask: “connect GitHub”"),
            _ => tr("Search or ask…"),
        }));
        match id {
            "permissoes" => self.permissoes.recarregar(),
            "memoria" => self.memoria.recarregar(),
            "rotinas" => {
                self.rotinas.render();
                self.rotinas.posicoes_debug();
            }
            "linha" => self.linha_tempo.recarregar(),
            "conversas" => self.conversas.mostrar(),
            "uso" => self.uso.render(),
            "ajustes" => self.ajustes.recarregar(),
            _ => {}
        }
        posicao_debug("busca", self.pedido.upcast_ref());
    }

    pub fn abrir(self: &Rc<Self>) {
        self.assinatura.borrow_mut().clear();
        self.render();
        self.janela.present();
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug central: aberta");
        }
    }

    /// The tasks now: the agent at work, the requests waiting, the ones that ended (most recent first).
    fn coletar(&self) -> Vec<Tarefa> {
        let Some(s) = self.shell.borrow().upgrade() else { return vec![] };
        let mut v = Vec::new();
        let conv = s.right_bar().conversa.clone();
        if let (Some((pedido, passos)), Some((inicio, tokens, pausado))) = (s.estado.agent_task.borrow().clone(), conv.work()) {
            let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
            v.push(Tarefa::Rodando { pedido, passos, inicio: agora - inicio.elapsed().as_secs() as i64, tokens, pausado });
        }
        if let Some(a) = s.agendador() {
            let r = crate::rotinas::Rotinas::carregar();
            for (id, (inicio, _)) in a.rodando.borrow().iter() {
                let nome = r.lista.iter().find(|x| x.id == *id).map(|x| x.nome.clone()).unwrap_or_default();
                v.push(Tarefa::Rotina { id: *id, nome, inicio: *inicio });
            }
        }
        for p in self.pendentes.borrow().iter() {
            let exe = p["exe"].as_str().and_then(|e| std::path::Path::new(e).file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let quem = if exe == "kalango-agente" { tr("Agent").to_string() } else if exe.is_empty() { p["caller"].as_str().unwrap_or("").to_string() } else { exe };
            let pedido = p["descricao"].as_str().filter(|d| !d.is_empty()).map(|d| d.split(['.', '\n']).next().unwrap_or(d).to_string())
                .unwrap_or_else(|| p["tool"].as_str().unwrap_or("").to_string());
            v.push(Tarefa::Esperando { id: p["id"].as_u64().unwrap_or(0), quem, pedido, desde: p["criado"].as_i64().unwrap_or(0), prazo: p["prazo_s"].as_u64().unwrap_or(120) });
        }
        for t in crate::conversa::tarefas(8) {
            let passos = t["passos"].as_array().into_iter().flatten()
                .map(|p| (p["marca"].as_str().unwrap_or("").to_string(), p["texto"].as_str().unwrap_or("").to_string(), p["t"].as_i64().unwrap_or(0))).collect();
            v.push(Tarefa::Fim {
                pedido: t["pedido"].as_str().unwrap_or("").to_string(), estado: t["estado"].as_str().unwrap_or("").to_string(),
                motivo: t["motivo"].as_str().unwrap_or("").to_string(), fim: t["fim"].as_i64().unwrap_or(0), tokens: t["tokens"].as_u64().unwrap_or(0), passos,
                rotina: t["rotina"].is_string(),
            });
        }
        v
    }

    fn render(self: &Rc<Self>) {
        let tarefas = self.coletar();
        let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
        let assinatura = format!("{tarefas:?}|{}|{}", self.escolhida.get(), agora / 60);
        if *self.assinatura.borrow() == assinatura {
            return;
        }
        *self.assinatura.borrow_mut() = assinatura;
        let ativos = tarefas.iter().filter(|t| matches!(t, Tarefa::Rodando { .. } | Tarefa::Rotina { .. })).count();
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug central: {} tarefas ({ativos} em execução)", tarefas.len());
        }
        let esperando = tarefas.iter().filter(|t| matches!(t, Tarefa::Esperando { .. })).count();
        let a = if ativos == 1 { tr("1 agent").to_string() } else { trf("{n} agents", &[("n", ativos.to_string())]) };
        self.subtitulo.set_text(&if esperando > 0 { format!("{a} · {}", if esperando == 1 { tr("1 waits for you").to_string() } else { trf("{n} wait for you", &[("n", esperando.to_string())]) }) } else { a });
        while let Some(x) = self.cartoes.first_child() {
            self.cartoes.remove(&x);
        }
        if tarefas.is_empty() {
            let v = rotulo(tr("No agent is working and no task ended yet. Ask for one above."), "desk-metrica-sub");
            v.set_xalign(0.0);
            self.cartoes.attach(&v, 0, 0, 2, 1);
        }
        let escolhida = self.escolhida.get().min(tarefas.len().saturating_sub(1));
        for (i, t) in tarefas.iter().enumerate() {
            let card = self.cartao(t, agora, i, i == escolhida);
            let clique = gtk::GestureClick::new();
            let w = Rc::downgrade(self);
            clique.connect_released(move |_, _, _, _| {
                if let Some(c) = w.upgrade() {
                    c.escolhida.set(i);
                    c.render();
                }
            });
            card.add_controller(clique);
            self.cartoes.attach(&card, (i % 2) as i32, (i / 2) as i32, 1, 1);
        }
        // the chosen task's steps
        while let Some(x) = self.linha.first_child() {
            self.linha.remove(&x);
        }
        let Some(t) = tarefas.get(escolhida) else {
            self.linha_titulo.set_text(tr("STEPS"));
            return;
        };
        let (nome, passos): (String, Vec<(String, String, i64)>) = match t {
            Tarefa::Rodando { .. } => {
                let conv = self.shell.borrow().upgrade().map(|s| s.right_bar().conversa.clone());
                (format!("{} · {}", tr("AGENT"), tr("RUNNING")), conv.map(|c| c.passos(12).into_iter().map(|(m, t, h)| (m.to_string(), t, h)).collect()).unwrap_or_default())
            }
            Tarefa::Rotina { nome, inicio, .. } => (format!("{} · {}", tr("ROUTINE"), tr("RUNNING")), vec![("◌".into(), nome.clone(), *inicio)]),
            Tarefa::Esperando { quem, pedido, desde, .. } => (format!("{} · {}", quem.to_uppercase(), tr("WAITING")), vec![("!".into(), pedido.clone(), *desde)]),
            Tarefa::Fim { estado, passos, rotina, .. } => (format!("{} · {}", if *rotina { tr("ROUTINE") } else { tr("AGENT") }, if estado == "concluido" { tr("DONE") } else if estado == "falhou" { tr("FAILED") } else { tr("STOPPED") }), passos.clone()),
        };
        self.linha_titulo.set_text(&nome);
        if passos.is_empty() {
            let v = rotulo(tr("No tool was used."), "desk-metrica-sub");
            v.set_xalign(0.0);
            self.linha.append(&v);
        }
        for (m, texto, h) in passos {
            let l = caixa(Orientation::Horizontal, 10, "");
            let ponto = caixa(Orientation::Vertical, 0, "central-marcador");
            ponto.add_css_class(match m.as_str() { "✓" => "feito", "!" => "espera", "✗" => "falhou", _ => "rodando" });
            ponto.set_valign(Align::Start);
            ponto.set_margin_top(5);
            l.append(&ponto);
            let col = caixa(Orientation::Vertical, 1, "");
            let a = rotulo(&texto, "central-passo");
            a.set_xalign(0.0);
            a.set_wrap(true);
            a.set_max_width_chars(36);
            col.append(&a);
            col.append(&{
                let b = rotulo(&hora_s(h), "central-passo-hora");
                b.set_xalign(0.0);
                b
            });
            l.append(&col);
            self.linha.append(&l);
        }
    }

    fn cartao(self: &Rc<Self>, t: &Tarefa, agora: i64, indice: usize, escolhida: bool) -> gtk::Box {
        let (nome, sub, estado, classe, titulo, progresso, meta) = match t {
            Tarefa::Rodando { pedido, passos, inicio, tokens, pausado } => (tr("Agent").to_string(), tr("local agent").to_string(),
                if *pausado { tr("Paused") } else { tr("Running") }, if *pausado { "pausado" } else { "executando" }, pedido.clone(),
                ((*passos as f64 + 1.0) / (*passos as f64 + 3.0)).min(0.95),
                format!("{} · {} · {}", if *passos == 1 { tr("1 step").to_string() } else { trf("{n} steps", &[("n", passos.to_string())]) },
                    trf("{n} min", &[("n", ((agora - inicio).max(0) / 60).to_string())]), trf("{n} tokens", &[("n", mil(*tokens))]))),
            Tarefa::Esperando { quem, pedido, desde, prazo, .. } => (quem.clone(), tr("local agent").to_string(), tr("Waiting"), "aguardando", pedido.clone(),
                ((agora - desde).max(0) as f64 / *prazo as f64).min(1.0),
                trf("waiting for approval for {t}", &[("t", if agora - desde < 60 { tr("less than 1 min").to_string() } else { trf("{n} min", &[("n", ((agora - desde) / 60).to_string())]) })])),
            Tarefa::Rotina { nome, inicio, .. } => (tr("Routine").to_string(), tr("local agent").to_string(), tr("Running"), "executando", nome.clone(), 0.5,
                trf("started at {time}", &[("time", hora(*inicio))])),
            Tarefa::Fim { pedido, estado, motivo, fim, tokens, rotina, .. } => {
                let ok = estado == "concluido";
                (if *rotina { tr("Routine") } else { tr("Agent") }.to_string(), tr("local agent").to_string(), if ok { tr("Completed") } else if estado == "falhou" { tr("Failed") } else { tr("Stopped") },
                    if ok { "concluido" } else if estado == "falhou" { "falhou" } else { "pausado" }, pedido.clone(), if ok { 1.0 } else { 0.35 },
                    if ok { format!("{} · {}", trf("done at {time}", &[("time", hora(*fim))]), trf("{n} tokens", &[("n", mil(*tokens))])) }
                    else if !motivo.is_empty() { motivo.clone() } else { trf("stopped at {time}", &[("time", hora(*fim))]) })
            }
        };
        let c = caixa(Orientation::Vertical, 10, "visao-tarefa");
        if escolhida {
            c.add_css_class("escolhida");
        }
        let cab = caixa(Orientation::Horizontal, 8, "");
        // (centred across, so nothing expands the card)
        let av = caixa(Orientation::Horizontal, 0, "visao-tarefa-avatar");
        av.set_size_request(28, 28);
        av.set_valign(Align::Center);
        av.set_hexpand(false);
        let sp = estrela(14, "#A5A1E2");
        sp.set_hexpand(true);
        sp.set_halign(Align::Center);
        sp.set_valign(Align::Center);
        av.append(&sp);
        cab.append(&av);
        let col = caixa(Orientation::Vertical, 1, "");
        col.set_hexpand(true);
        for (t, k) in [(nome.as_str(), "visao-tarefa-nome"), (sub.as_str(), "visao-tarefa-sub")] {
            let l = rotulo(t, k);
            l.set_xalign(0.0);
            col.append(&l);
        }
        cab.append(&col);
        let chip = rotulo(estado, "visao-tarefa-estado");
        chip.add_css_class(classe);
        chip.set_valign(Align::Center);
        cab.append(&chip);
        c.append(&cab);
        let tl = rotulo(&titulo, "visao-tarefa-titulo");
        tl.set_xalign(0.0);
        tl.set_ellipsize(gtk::pango::EllipsizeMode::End);
        tl.set_max_width_chars(1);
        tl.set_hexpand(true);
        c.append(&tl);
        let (b, s) = barra(200, match classe { "executando" => "#8ED2C3", "aguardando" => "#E5C98C", "falhou" => "#EFA2A6", _ => "#8E8F99" });
        b.set_hexpand(true);
        b.set_size_request(-1, 3);
        c.append(&b);
        s.ir(progresso, &b);
        let linha = caixa(Orientation::Horizontal, 8, "");
        let m = rotulo(&meta, "visao-tarefa-meta");
        m.set_xalign(0.0);
        m.set_hexpand(true);
        m.set_ellipsize(gtk::pango::EllipsizeMode::End);
        m.set_max_width_chars(1);
        linha.append(&m);
        let botao = |glifo: &str, dica: &str| {
            let b = gtk::Button::new();
            b.set_child(Some(&ui_icon(glifo, "#E2E2E6", 14)));
            b.add_css_class("visao-tarefa-botao");
            b.set_tooltip_text(Some(dica));
            b
        };
        let shell = self.shell.borrow().clone();
        match t {
            Tarefa::Rodando { pausado, .. } => {
                let p = botao(if *pausado { "tocar" } else { "pausa" }, if *pausado { tr("Resume") } else { tr("Pause") });
                let (s2, pausado) = (shell.clone(), *pausado);
                p.connect_clicked(move |_| {
                    if let Some(s) = s2.upgrade() {
                        s.right_bar().conversa.pause(!pausado);
                    }
                });
                linha.append(&p);
                let parar = botao("parar", tr("Stop"));
                parar.connect_clicked(move |_| {
                    if let Some(s) = shell.upgrade() {
                        let c = s.right_bar().conversa.clone();
                        c.pause(false);
                        c.stop(true);
                    }
                });
                linha.append(&parar);
            }
            Tarefa::Rotina { id, .. } => {
                let parar = botao("parar", tr("Stop"));
                let id = *id;
                parar.connect_clicked(move |_| {
                    if let Some(a) = shell.upgrade().and_then(|s| s.agendador()) {
                        a.parar(id);
                    }
                });
                linha.append(&parar);
            }
            Tarefa::Esperando { id, .. } => {
                let ver = botao("olho", tr("See the request"));
                let s2 = shell.clone();
                ver.connect_clicked(move |_| {
                    if let Some(s) = s2.upgrade() {
                        s.right_bar().abrir(crate::lateral::Modo::Agente);
                    }
                });
                linha.append(&ver);
                let recusar = botao("parar", tr("Refuse"));
                let id = *id;
                recusar.connect_clicked(move |_| {
                    if let Err(e) = crate::servicos::kalangod_decidir(id, false) {
                        eprintln!("kalango-shell: recusar o pedido {id}: {e}");
                    }
                });
                linha.append(&recusar);
            }
            Tarefa::Fim { .. } => {
                // its steps on the right
                let passos = botao("menu", tr("Steps"));
                let w = Rc::downgrade(self);
                passos.connect_clicked(move |_| {
                    if let Some(c) = w.upgrade() {
                        c.escolhida.set(indice);
                        c.render();
                    }
                });
                linha.append(&passos);
            }
        }
        c.append(&linha);
        c
    }
}
