//! Barras laterais do sistema (página "22 · Barras laterais"): presas às bordas da tela, não dentro de janelas.
//!
//! - Direita, três modos na mesma superfície (trocar de modo desliza o conteúdo, a barra não fecha):
//!   **Avisos** (central de notificações, calendário, não perturbe), **Controles** (Wi-Fi, Bluetooth, não perturbe,
//!   modo avião, volume, brilho, modo de energia, mídia, dispositivos) e **Agente** (pedidos e linha do tempo do
//!   kalangod).
//! - Esquerda, **Hoje**: resumo dos agentes, foco (cronômetro), CPU e GPU ao vivo, mídia.
//!
//! Movimento: entram deslizando 24 px da borda com opacidade (mola suave) e o conteúdo em cascata de 24 ms;
//! fecham com a mola rápida; alternâncias respondem com a mola elástica.

use crate::conversa::{ui_icon, Conversa};
use crate::{agenda, clima};
use crate::midia::{self, Midia};
use crate::notificacoes;
use crate::servicos;
use crate::sistema::{self, Sistema};
use crate::superficies::{caixa, camada, rotulo, BARRA_ALTURA};
use crate::Shell;
use gtk::prelude::*;
use kalango_i18n::{n_, tr, trf};
use gtk::{gdk, gio, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_movimento::{self as mv, Animavel, Canal};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

pub const LARGURA: i32 = 400;

/// The bars' width (Settings › Appearance › Dock and multitasking: narrow, standard, wide).
pub fn largura() -> i32 {
    match crate::configuracoes::ajustes()["barras_largura"].as_str() {
        Some("estreita") => 340,
        Some("larga") => 480,
        _ => LARGURA,
    }
}

/// What clicking the icon of a slider does (e.g. mute).
type IconAction = Box<dyn Fn(&Rc<Direita>)>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Modo {
    Avisos,
    Controles,
    Agente,
}

impl Modo {
    fn nome(&self) -> &'static str {
        match self {
            Modo::Avisos => "avisos",
            Modo::Controles => "controles",
            Modo::Agente => "agente",
        }
    }
}

fn icone(nome: &str, px: i32) -> gtk::Image {
    let i = gtk::Image::from_icon_name(nome);
    i.set_pixel_size(px);
    i
}

fn botao_icone(nome: &str, dica: &str, classe: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.set_child(Some(&icone(nome, 16)));
    b.add_css_class("botao-icone");
    if !classe.is_empty() {
        b.add_css_class(classe);
    }
    b.set_tooltip_text(Some(dica));
    b
}

fn limpar(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

fn cartao(classe: &str) -> gtk::Box {
    let c = caixa(Orientation::Vertical, 8, "cartao-lateral");
    if !classe.is_empty() {
        c.add_css_class(classe);
    }
    c
}

/// The screen's height (the first monitor's), 1080 when unknown.
fn altura_da_tela() -> i32 {
    gdk::Display::default().and_then(|d| d.monitors().item(0)).and_downcast::<gdk::Monitor>().map(|m| m.geometry().height()).unwrap_or(1080)
}

fn secao(t: &str) -> gtk::Label {
    let l = rotulo(t, "caps");
    l.set_halign(Align::Start);
    l
}

pub(crate) fn tempo_relativo(unix: i64) -> String {
    let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(unix);
    match (agora - unix).max(0) {
        s if s < 60 => tr("now").into(),
        s if s < 3600 => trf("{n} min", &[("n", (s / 60).to_string())]),
        s if s < 86400 => trf("{n} h", &[("n", (s / 3600).to_string())]),
        s => trf("{n} d", &[("n", (s / 86400).to_string())]),
    }
}

/// Executa `f` fora da thread da interface e entrega o resultado em `depois` (na thread da interface).
pub fn em_segundo_plano<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static, depois: impl FnOnce(T) + 'static) {
    glib::MainContext::default().spawn_local(async move {
        if let Ok(v) = gio::spawn_blocking(f).await {
            depois(v);
        }
    });
}

/// Superfície presa a uma borda, que entra deslizando e sai rápido.
struct Painel {
    janela: gtk::ApplicationWindow,
    corpo: Animavel,
    aberta: Cell<bool>,
    lado: f64,
    /// the whole side of the screen (the right bar, as a notification centre): flush with the edge, from under the
    /// top bar to the bottom; else a floating card 8 px from the edges
    inteira: bool,
    borda: Edge,
    /// Pinned on a wide screen (design 87844), the bar is flush with the edge from under the top bar to the bottom —
    /// past the dock's zone, which a surface that reserves room of its own would respect. So the room for the
    /// windows is reserved by this other, invisible surface, and the bar itself reserves nothing.
    reserva: gtk::ApplicationWindow,
}

impl Painel {
    fn new(app: &gtk::Application, nome: &str, direita: bool, filho: &gtk::Box, inteira: bool) -> Painel {
        let borda = if direita { Edge::Right } else { Edge::Left };
        let janela = camada(app, nome, Layer::Top, &[Edge::Top, Edge::Bottom, borda]);
        let m = if inteira { 0 } else { 8 };
        janela.set_margin(Edge::Top, m);
        janela.set_margin(Edge::Bottom, m);
        janela.set_margin(borda, m);
        if inteira {
            filho.add_css_class("inteira");
        }
        if !direita {
            filho.add_css_class("esquerda");
        }
        filho.set_size_request(largura(), -1);
        // fixed width, as in the design: the window keeps its default width and only grows if the content's
        // minimum demands it (a long text has a large natural width and would otherwise widen the bar)
        janela.set_default_size(largura(), -1);
        let corpo = Animavel::new(filho);
        janela.set_child(Some(&corpo));
        let reserva = camada(app, &format!("{nome}-reserva"), Layer::Background, &[Edge::Top, Edge::Bottom, borda]);
        reserva.add_css_class("lateral-reserva");
        reserva.set_default_size(1, 1);
        Painel { janela, corpo, aberta: Cell::new(false), lado: if direita { 24.0 } else { -24.0 }, inteira, borda, reserva }
    }

    /// As in the design, the bar runs from under the top bar to the bottom edge (a floating one: to 8 px from it),
    /// past the dock's reserved zone. On narrow screens (below 1600 px) that would cover the end of the centered
    /// dock, so there it stops above the dock. `pinned`: it reserves its own width, so it stays within the free area.
    ///
    /// With the dock as a taskbar (the whole bottom, and no top bar) both bars are sides of the screen: flush with
    /// the edge, from the top to the taskbar — the compositor keeps them out of the taskbar's reserved zone.
    fn place(&self, pinned: bool) {
        let wide = gdk::Display::default().and_then(|d| d.monitors().item(0)).and_downcast::<gdk::Monitor>()
            .map(|m| m.geometry().width() >= 1600).unwrap_or(false);
        let taskbar = crate::superficies::modo_barra_de_tarefas();
        // pinned on a wide screen: flush with the edge, no margin nor corners (design 87844)
        let flush = pinned && wide && !taskbar;
        let inteira = self.inteira || taskbar || flush;
        let m = if inteira { 0 } else { 8 };
        if flush {
            self.reserva.set_exclusive_zone(largura());
            self.reserva.present();
        } else {
            self.reserva.set_exclusive_zone(0);
            self.reserva.set_visible(false);
        }
        self.janela.set_margin(Edge::Bottom, m);
        self.janela.set_margin(self.borda, m);
        if let Some(f) = self.corpo.first_child() {
            if inteira { f.add_css_class("inteira") } else { f.remove_css_class("inteira") }
        }
        if taskbar {
            self.janela.set_exclusive_zone(if pinned { largura() } else { 0 });
            self.janela.set_margin(Edge::Top, 0);
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug lateral: {} inteira, do topo até a barra de tarefas", if self.borda == Edge::Right { "direita" } else { "esquerda" });
            }
        } else if wide {
            self.janela.set_exclusive_zone(-1);
            self.janela.set_margin(Edge::Top, BARRA_ALTURA + m);
            if flush && std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug lateral: {} fixada rente à borda, da barra superior até embaixo", if self.borda == Edge::Right { "direita" } else { "esquerda" });
            }
        } else {
            self.janela.set_exclusive_zone(if pinned { largura() + m } else { 0 });
            self.janela.set_margin(Edge::Top, m);
        }
    }

    /// The width chosen now (the bar changes at once, open or not).
    fn aplicar_largura(&self, pinned: bool) {
        if let Some(f) = self.corpo.first_child() {
            f.set_size_request(largura(), -1);
        }
        self.janela.set_default_size(largura(), -1);
        self.place(pinned);
    }

    fn abrir(&self) {
        if self.aberta.replace(true) {
            return;
        }
        self.place(false);
        self.janela.present();
        self.janela.set_keyboard_mode(KeyboardMode::OnDemand);
        self.corpo.definir(Canal::X, self.lado).definir(Canal::Opacidade, 0.0);
        self.corpo.ir(Canal::X, 0.0, mv::SUAVE).ir(Canal::Opacidade, 1.0, mv::SUAVE);
    }

    fn fechar(&self) {
        if !self.aberta.replace(false) {
            return;
        }
        self.janela.set_keyboard_mode(KeyboardMode::None);
        let j = self.janela.downgrade();
        let corpo = self.corpo.downgrade();
        self.corpo.sair((self.lado, 0.0), 1.0, move || {
            // reaberta durante a saída? então fica
            if let (Some(j), Some(c)) = (j.upgrade(), corpo.upgrade()) {
                if c.valor(Canal::Opacidade) < 0.05 {
                    j.set_visible(false);
                }
            }
        });
    }
}

// ================================================================================================= barra direita

pub struct Direita {
    painel: Painel,
    modo: Cell<Modo>,
    abas: Vec<(Modo, gtk::ToggleButton)>,
    paginas: gtk::Stack,
    avisos: gtk::Box,
    controles: gtk::Box,
    controles_rodape: gtk::Box,
    agente: gtk::Box,
    pub(crate) conversa: Rc<Conversa>,
    shell: RefCell<Weak<Shell>>,
    sistema: RefCell<Sistema>,
    mes: Cell<(i32, i32)>,
    atualizando: Cell<bool>,
    /// guards against rendering twice when a tab toggle re-enters `mostrar`
    switching: Cell<bool>,
    /// pinned: the bar keeps its space (windows go beside it) and stays open
    fixada: Cell<bool>,
    alfinete: gtk::Button,
    /// web pages the agents read today (the footer says nothing went to the cloud only when this is 0)
    web_hoje: Cell<usize>,
    avisos_rodape: gtk::Box,
    avisos_topo: gtk::Box,
    /// the calendar under the date, open or closed by the person; until then it is open on a tall screen (a short
    /// one keeps the room for the notices)
    calendario_aberto: Cell<Option<bool>>,
    /// all notices listed ("See all N"), or only the latest groups under the summary
    avisos_todos: Cell<bool>,
    /// the local model's summary of the unread notices: (their ids, the points; None while it is being made)
    resumo_avisos: RefCell<(Vec<u32>, Option<Vec<String>>)>,
    /// apps whose group is expanded ("+3 from Discord")
    grupos_abertos: RefCell<std::collections::HashSet<String>>,
}

impl Direita {
    pub fn new(app: &gtk::Application) -> Rc<Direita> {
        let raiz = caixa(Orientation::Vertical, 10, "lateral");
        // cabeçalho: modos, fixar, fechar
        let topo = caixa(Orientation::Horizontal, 6, "");
        let seg = caixa(Orientation::Horizontal, 0, "segmentos");
        seg.set_hexpand(true);
        let mut abas = Vec::new();
        let mut primeiro: Option<gtk::ToggleButton> = None;
        for (m, t) in [(Modo::Avisos, tr("Notices")), (Modo::Controles, tr("Controls")), (Modo::Agente, tr("Agent"))] {
            let b = gtk::ToggleButton::with_label(t);
            b.set_hexpand(true);
            if let Some(p) = &primeiro {
                b.set_group(Some(p));
            } else {
                primeiro = Some(b.clone());
            }
            seg.append(&b);
            abas.push((m, b));
        }
        topo.append(&seg);
        let alfinete = gtk::Button::new();
        alfinete.set_child(Some(&ui_icon("alfinete", MUTED, 28)));
        alfinete.add_css_class("fantasma");
        alfinete.add_css_class("alfinete");
        alfinete.set_tooltip_text(Some(tr("Pin: keep the bar open beside the windows")));
        topo.append(&alfinete);
        let fechar = gtk::Button::new();
        fechar.set_child(Some(&ui_icon("fechar", MUTED, 16)));
        fechar.add_css_class("fantasma");
        fechar.set_tooltip_text(Some(tr("Close (Esc)")));
        topo.append(&fechar);
        raiz.append(&topo);

        let paginas = gtk::Stack::new();
        paginas.set_vexpand(true);
        // Notices, as a notification centre: the title and "Clear all" on top, the notices scroll, and the date with
        // the calendar and the quick buttons stay at the bottom of the bar
        let avisos = caixa(Orientation::Vertical, 12, "");
        let avisos_topo = caixa(Orientation::Horizontal, 8, "lateral-titulo-linha");
        let avisos_rodape = caixa(Orientation::Vertical, 12, "lateral-base");
        {
            let r = gtk::ScrolledWindow::new();
            r.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
            r.set_vexpand(true);
            r.set_child(Some(&avisos));
            let pagina = caixa(Orientation::Vertical, 12, "");
            pagina.append(&avisos_topo);
            pagina.append(&r);
            pagina.append(&avisos_rodape);
            paginas.add_named(&pagina, Some("avisos"));
        }
        // Controls: the content scrolls, the footer stays at the bottom of the bar
        let controles = caixa(Orientation::Vertical, 12, "");
        let controles_rodape = caixa(Orientation::Vertical, 0, "");
        {
            let r = gtk::ScrolledWindow::new();
            r.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
            r.set_vexpand(true);
            r.set_child(Some(&controles));
            let pagina = caixa(Orientation::Vertical, 12, "");
            pagina.append(&r);
            pagina.append(&controles_rodape);
            paginas.add_named(&pagina, Some("controles"));
        }
        // Agent mode: the conversation (its own scrolling, the composer stays at the bottom); before the first
        // message it shows the pending requests and the timeline (`agente`)
        let conversa = Conversa::new();
        conversa.modo_lateral();
        paginas.add_named(&conversa.raiz, Some("agente"));
        let agente = conversa.vazio.clone();
        raiz.append(&paginas);

        let painel = Painel::new(app, "kalango-barra-lateral-direita", true, &raiz, true);
        let hoje = glib::DateTime::now_local().unwrap();
        let d = Rc::new(Direita {
            painel, modo: Cell::new(Modo::Avisos), abas, paginas, avisos, controles, controles_rodape, agente, conversa, shell: RefCell::default(),
            sistema: RefCell::default(), mes: Cell::new((hoje.year(), hoje.month())), atualizando: Cell::new(false), switching: Cell::new(false),
            fixada: Cell::new(false), alfinete: alfinete.clone(), web_hoje: Cell::new(0),
            avisos_rodape, avisos_topo, calendario_aberto: Cell::new(None), avisos_todos: Cell::new(false),
            resumo_avisos: RefCell::default(), grupos_abertos: RefCell::default(),
        });
        let w = Rc::downgrade(&d);
        alfinete.connect_clicked(move |_| {
            if let Some(d) = w.upgrade() {
                d.set_pinned(!d.fixada.get());
            }
        });
        for (m, b) in &d.abas {
            let w = Rc::downgrade(&d);
            let m = *m;
            b.connect_toggled(move |b| {
                if b.is_active() {
                    if let Some(d) = w.upgrade() {
                        if !d.switching.get() {
                            d.mostrar(m);
                        }
                    }
                }
            });
        }
        let w = Rc::downgrade(&d);
        fechar.connect_clicked(move |_| {
            if let Some(d) = w.upgrade() {
                d.close();
            }
        });
        let teclas = gtk::EventControllerKey::new();
        let w = Rc::downgrade(&d);
        teclas.connect_key_pressed(move |_, k, _, _| {
            if k == gdk::Key::Escape {
                if let Some(d) = w.upgrade() {
                    d.close();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        d.painel.janela.add_controller(teclas);
        // enquanto aberta, os Controles leem o sistema a cada 2 s
        let w = Rc::downgrade(&d);
        glib::timeout_add_local(Duration::from_secs(2), move || {
            let Some(d) = w.upgrade() else { return glib::ControlFlow::Break };
            if d.painel.aberta.get() && d.modo.get() != Modo::Agente {
                d.ler_sistema();
            }
            glib::ControlFlow::Continue
        });
        d
    }

    pub fn ligar(self: &Rc<Self>, shell: &Rc<Shell>) {
        *self.shell.borrow_mut() = Rc::downgrade(shell);
        // the agent picker's other choices go to the agents center: run a flow, create or manage agents
        let s = Rc::downgrade(shell);
        *self.conversa.on_picked_other.borrow_mut() = Some(Box::new(move |x| {
            let Some(s) = s.upgrade() else { return };
            match x {
                crate::agent_picker::Picked::Run(a) => {
                    s.abrir_central(None);
                    if let Some(c) = s.central() {
                        c.executar_fluxo(a["config"]["flow_id"].as_str().unwrap_or(""));
                    }
                }
                crate::agent_picker::Picked::Create => s.abrir_central(Some("novo-agente")),
                crate::agent_picker::Picked::Manage => s.abrir_central(Some("agentes")),
                crate::agent_picker::Picked::Agent(_) => {}
            }
        }));
        let s = Rc::downgrade(shell);
        self.conversa.on_busy(move |activity| {
            if let Some(s) = s.upgrade() {
                s.estado.agent_working.set(activity.is_some());
                *s.estado.agent_task.borrow_mut() = activity;
                s.estado.avisar();
            }
        });
        let w = Rc::downgrade(self);
        shell.notificacoes().ouvir(move || {
            if let Some(d) = w.upgrade() {
                if d.painel.aberta.get() && d.modo.get() == Modo::Avisos {
                    d.render_avisos(false);
                }
            }
        });
        let w = Rc::downgrade(self);
        shell.midia.ouvir(move |_| {
            if let Some(d) = w.upgrade() {
                if d.painel.aberta.get() && d.modo.get() == Modo::Controles {
                    d.render_controles(false);
                }
            }
        });
    }

    fn shell(&self) -> Option<Rc<Shell>> {
        self.shell.borrow().upgrade()
    }

    /// Closes the bar and lets the notifications know (banners go back to the corner).
    /// Pinned, the bar reserves its width on the screen edge (the windows rearrange beside it) and stays open.
    fn set_pinned(&self, on: bool) {
        self.fixada.set(on);
        self.painel.place(on);
        if on { self.alfinete.add_css_class("ativo") } else { self.alfinete.remove_css_class("ativo") }
        crate::conversa::set_icon(&self.alfinete.child().and_downcast::<gtk::Image>().unwrap(), "alfinete", if on { "#A5A1E2" } else { MUTED });
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug lateral: fixada={on}");
        }
    }

    /// Opens it pinned (wide screens: Settings › Appearance › "Fixar por padrão em telas largas").
    pub fn pin_open(self: &Rc<Self>) {
        self.abrir(self.modo.get());
        self.set_pinned(true);
    }

    /// The width changed in Settings.
    pub fn width_changed(&self) {
        self.painel.aplicar_largura(self.fixada.get());
    }

    pub fn close(&self) {
        if self.fixada.get() {
            self.set_pinned(false);
        }
        self.painel.fechar();
        if let Some(s) = self.shell() {
            s.notificacoes().set_side_bar(false, false);
        }
    }

    /// Closes the bar if it floats over the windows (open and not pinned: a pinned bar has its own room).
    pub fn close_if_floating(&self) {
        if self.painel.aberta.get() && !self.fixada.get() {
            self.close();
        }
    }

    pub fn aberta(&self) -> bool {
        self.painel.aberta.get()
    }

    /// The bar is open showing the agent's conversation.
    pub fn mostrando_agente(&self) -> bool {
        self.painel.aberta.get() && self.modo.get() == Modo::Agente
    }

    /// Opens the Agent mode and sends the request (from the command bar).
    pub fn ask_agent(self: &Rc<Self>, request: &str) {
        self.abrir(Modo::Agente);
        self.conversa.ask(request);
    }

    pub fn alternar(self: &Rc<Self>, m: Modo) {
        if self.painel.aberta.get() && self.modo.get() == m {
            self.close();
        } else {
            self.abrir(m);
        }
    }

    pub fn abrir(self: &Rc<Self>, m: Modo) {
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug lateral: aberta ({})", match m { Modo::Avisos => "avisos", Modo::Controles => "controles", Modo::Agente => "agente" });
        }
        let ja = self.painel.aberta.get();
        self.painel.abrir();
        if !ja || self.modo.get() != m {
            self.mostrar(m);
        }
    }

    fn mostrar(self: &Rc<Self>, m: Modo) {
        let antes = self.modo.replace(m);
        self.switching.set(true);
        for (x, b) in &self.abas {
            if *x == m && !b.is_active() {
                b.set_active(true);
            }
        }
        self.switching.set(false);
        if let Some(s) = self.shell() {
            s.notificacoes().set_side_bar(true, m == Modo::Avisos);
        }
        self.paginas.set_visible_child_name(m.nome());
        match m {
            Modo::Avisos => {
                self.render_avisos(true);
                self.ler_sistema();
            }
            Modo::Controles => {
                self.render_controles(true);
                self.ler_sistema();
            }
            Modo::Agente => {
                if self.conversa.is_empty() {
                    self.render_agente();
                }
                self.conversa.focus();
            }
        }
        // o conteúdo novo desliza na horizontal (mola rápida), do lado do modo de onde veio
        if antes != m {
            let dir = if (m as u8) > (antes as u8) { 32.0 } else { -32.0 };
            let pagina = match m {
                Modo::Avisos => &self.avisos,
                Modo::Controles => &self.controles,
                Modo::Agente => &self.agente,
            };
            let mut c = pagina.first_child();
            while let Some(w) = c {
                if let Some(a) = w.downcast_ref::<Animavel>() {
                    a.definir(Canal::X, dir);
                    a.ir(Canal::X, 0.0, mv::RAPIDA);
                }
                c = w.next_sibling();
            }
        }
    }

    /// Itens de uma página entram em cascata (24 ms).
    fn montar(&self, pagina: &gtk::Box, blocos: Vec<gtk::Widget>, cascata: bool) {
        limpar(pagina);
        let mut animaveis = Vec::new();
        for b in blocos {
            let a = Animavel::new(&b);
            pagina.append(&a);
            animaveis.push(a);
        }
        if cascata {
            mv::cascata(&animaveis, 8.0);
        }
    }

    // ------------------------------------------------------------------------------------------- Avisos

    fn render_avisos(self: &Rc<Self>, cascata: bool) {
        let Some(shell) = self.shell() else { return };
        let n = shell.notificacoes();
        let hist = n.historico.borrow().clone();
        let mut blocos: Vec<gtk::Widget> = Vec::new();

        // the local model's summary when several notices wait (design: "12 avisos resumidos em 3 pontos")
        let nao_lidas: Vec<&notificacoes::Guardado> = hist.iter().filter(|g| !g.lida).collect();
        let com_resumo = nao_lidas.len() >= RESUMO_MINIMO;
        if com_resumo {
            blocos.push(self.resumo_avisos(&n, &nao_lidas).upcast());
        }

        // the title and "Clear all", fixed above the list (as a notification centre's)
        limpar(&self.avisos_topo);
        let t = rotulo(tr("Notifications"), "lateral-titulo");
        t.set_hexpand(true);
        t.set_halign(Align::Start);
        self.avisos_topo.append(&t);
        if !hist.is_empty() {
            let l = gtk::Button::with_label(tr("Clear all"));
            l.add_css_class("link");
            let nn = n.clone();
            l.connect_clicked(move |_| nn.limpar());
            self.avisos_topo.append(&l);
        }
        if hist.is_empty() {
            let v = cartao("vazio-lateral");
            v.append(&ui_icon("check", MUTED, 20));
            v.append(&rotulo(tr("All caught up. Notices that leave the screen are kept here."), "fraco"));
            blocos.push(v.upcast());
        }
        // one card per app: the latest notice, and "+N from <app>" for the others
        let mut grupos: Vec<(String, Vec<notificacoes::Guardado>)> = Vec::new();
        for g in &hist {
            match grupos.iter_mut().find(|(a, _)| *a == g.app) {
                Some((_, v)) => v.push(g.clone()),
                None => grupos.push((g.app.clone(), vec![g.clone()])),
            }
        }
        // under the summary only the latest groups show, until "See all N"
        let limite = if com_resumo && !self.avisos_todos.get() { 2 } else { 12 };
        for (app, itens) in grupos.iter().take(limite) {
            blocos.push(self.grupo_avisos(&n, app, itens).upcast());
        }
        self.montar(&self.avisos, blocos, cascata);
        self.render_base();
    }

    /// The bottom of the notification centre: today's date (it opens and closes the calendar), the calendar, and the
    /// quick buttons (Wi-Fi, Bluetooth, Do not disturb, Night light — the rest is in Controls).
    fn render_base(self: &Rc<Self>) {
        let Some(shell) = self.shell() else { return };
        let s = self.sistema.borrow().clone();
        limpar(&self.avisos_rodape);
        let data = gtk::Button::new();
        data.add_css_class("lateral-data");
        let linha = caixa(Orientation::Horizontal, 8, "");
        let hoje = glib::DateTime::now_local().unwrap();
        // "%A, %B %-d" in English; each language says the date its way ("domingo, 15 de novembro")
        let texto = hoje.format(tr("%A, %B %-d")).map(|s| s.to_string()).unwrap_or_default();
        let l = rotulo(&texto, "lateral-data-texto");
        l.set_hexpand(true);
        l.set_halign(Align::Start);
        linha.append(&l);
        let aberto = self.calendario_aberto();
        linha.append(&ui_icon(if aberto { "chevron-baixo" } else { "seta-cima" }, MUTED, 16));
        data.set_child(Some(&linha));
        data.set_tooltip_text(Some(if aberto { tr("Hide the calendar") } else { tr("Show the calendar") }));
        let w = Rc::downgrade(self);
        data.connect_clicked(move |_| {
            if let Some(d) = w.upgrade() {
                let aberto = !d.calendario_aberto();
                d.calendario_aberto.set(Some(aberto));
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug avisos: calendário {}", if aberto { "aberto" } else { "fechado" });
                }
                d.render_base();
            }
        });
        crate::central::posicao_debug("avisos data", data.upcast_ref());
        self.avisos_rodape.append(&data);
        if aberto {
            self.avisos_rodape.append(&self.calendario());
        }
        let rapidos = caixa(Orientation::Horizontal, 8, "");
        rapidos.set_homogeneous(true);
        rapidos.append(&self.rapido("wifi", tr("Wi-Fi"), s.rede.wifi_ligado, |d, v| d.agir(move || sistema::wifi(v))));
        if s.bt.existe {
            rapidos.append(&self.rapido("bluetooth", tr("Bluetooth"), s.bt.ligado, |d, v| d.agir(move || sistema::bluetooth(v))));
        }
        let np = shell.notificacoes().nao_perturbe.get();
        rapidos.append(&self.rapido("sino-mudo", tr("Do not disturb"), np, |d, v| {
            if let Some(s) = d.shell() {
                s.notificacoes().definir_nao_perturbe(v);
            }
        }));
        // night light only where the compositor can warm the screen (the Controls tile says why when it cannot)
        if let (Some(on), false) = (s.luz_noturna, s.luz_noturna_indisponivel) {
            rapidos.append(&self.rapido("lua", tr("Night light"), on, |d, v| {
                d.agir(move || sistema::luz_noturna(v));
                if v {
                    let w = Rc::downgrade(d);
                    glib::timeout_add_seconds_local_once(sistema::until_morning() as u32, move || {
                        if let Some(d) = w.upgrade() {
                            d.agir(|| sistema::luz_noturna(false));
                        }
                    });
                }
            }));
        }
        self.avisos_rodape.append(&rapidos);
    }

    fn calendario_aberto(&self) -> bool {
        self.calendario_aberto.get().unwrap_or_else(|| altura_da_tela() >= 1000)
    }

    /// A quick button of the notification centre: the glyph over its name, filled with the accent when on.
    fn rapido(self: &Rc<Self>, glifo: &'static str, titulo: &str, ligado: bool, f: impl Fn(&Rc<Direita>, bool) + 'static) -> gtk::Button {
        let b = gtk::Button::new();
        b.add_css_class("rapido");
        b.set_widget_name(titulo);
        if ligado {
            b.add_css_class("ligado");
        }
        // as in the design: 11 px above the glyph, its name 9 px from the bottom of the 64 px button
        let col = caixa(Orientation::Vertical, 14, "");
        col.set_valign(Align::Center);
        let ic = ui_icon(glifo, if ligado { ON_INK } else { TEXT }, 16);
        col.append(&ic);
        let t = rotulo(titulo, "rapido-titulo");
        t.set_ellipsize(gtk::pango::EllipsizeMode::End);
        t.set_max_width_chars(1);
        t.set_hexpand(true);
        col.append(&t);
        let a = Animavel::new(&col);
        b.set_child(Some(&a));
        b.set_tooltip_text(Some(titulo));
        let w = Rc::downgrade(self);
        b.connect_clicked(move |b| {
            let novo = !b.has_css_class("ligado");
            if novo { b.add_css_class("ligado") } else { b.remove_css_class("ligado") }
            crate::conversa::set_icon(&ic, glifo, if novo { ON_INK } else { TEXT });
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug avisos: rápido {} → {novo}", b.widget_name());
            }
            a.definir(Canal::Escala, 0.94);
            a.ir(Canal::Escala, 1.0, mv::ELASTICA);
            if let Some(d) = w.upgrade() {
                f(&d, novo);
            }
        });
        crate::central::posicao_debug(&format!("avisos rapido {titulo}"), b.upcast_ref());
        b
    }

    /// The summary card: three points by the local model (made in the background, once per set of notices).
    fn resumo_avisos(self: &Rc<Self>, n: &Rc<notificacoes::Notificacoes>, nao_lidas: &[&notificacoes::Guardado]) -> gtk::Box {
        let ids: Vec<u32> = nao_lidas.iter().map(|g| g.id).collect();
        let pronto = {
            let r = self.resumo_avisos.borrow();
            (r.0 == ids).then(|| r.1.clone())
        };
        let pontos = match pronto {
            Some(p) => p,
            None => {
                // a new set: ask the local model (in the background) and draw again when it answers
                *self.resumo_avisos.borrow_mut() = (ids.clone(), None);
                let dados = serde_json::Value::Array(nao_lidas.iter().map(|g| serde_json::json!({"app": g.app, "titulo": g.titulo, "corpo": g.corpo})).collect());
                let w = Rc::downgrade(self);
                em_segundo_plano(move || resumir_avisos(&dados), move |pontos: Vec<String>| {
                    let Some(d) = w.upgrade() else { return };
                    if d.resumo_avisos.borrow().0 == ids {
                        d.resumo_avisos.borrow_mut().1 = Some(pontos);
                        if d.painel.aberta.get() && d.modo.get() == Modo::Avisos {
                            d.render_avisos(false);
                        }
                    }
                });
                None
            }
        };
        let c = cartao("resumo-avisos");
        c.set_spacing(7);
        let cab = caixa(Orientation::Horizontal, 6, "");
        cab.append(&ui_icon("spark", ACCENT, 16));
        cab.append(&secao(&trf("{n} NOTICES SUMMED UP IN 3 POINTS", &[("n", nao_lidas.len().to_string())])));
        c.append(&cab);
        match pontos {
            Some(p) => {
                for t in p {
                    let l = caixa(Orientation::Horizontal, 8, "");
                    let ponto = caixa(Orientation::Horizontal, 0, "resumo-marcador");
                    ponto.set_valign(Align::Center);
                    l.append(&ponto);
                    let r = rotulo(&t, "resumo-ponto");
                    r.set_xalign(0.0);
                    r.set_wrap(true);
                    r.set_max_width_chars(1);
                    r.set_hexpand(true);
                    l.append(&r);
                    c.append(&l);
                }
            }
            None => {
                let r = rotulo(tr("Summing up with the local model…"), "fraco");
                r.set_halign(Align::Start);
                c.append(&r);
            }
        }
        let acoes = caixa(Orientation::Horizontal, 8, "");
        let ver = gtk::Button::with_label(&if self.avisos_todos.get() { tr("Show fewer").to_string() } else { trf("See all {n}", &[("n", nao_lidas.len().to_string())]) });
        ver.add_css_class("resumo-ver");
        let w = Rc::downgrade(self);
        ver.connect_clicked(move |_| {
            if let Some(d) = w.upgrade() {
                d.avisos_todos.set(!d.avisos_todos.get());
                d.render_avisos(false);
            }
        });
        acoes.append(&ver);
        let lidos = gtk::Button::with_label(tr("Mark all as read"));
        lidos.add_css_class("fantasma");
        lidos.add_css_class("resumo-lidos");
        let nn = n.clone();
        lidos.connect_clicked(move |_| nn.marcar_todas_lidas());
        acoes.append(&lidos);
        c.append(&acoes);
        c
    }

    /// One app's notices: the latest as a full card, the others behind "+N from <app>".
    fn grupo_avisos(self: &Rc<Self>, n: &Rc<notificacoes::Notificacoes>, app: &str, itens: &[notificacoes::Guardado]) -> gtk::Box {
        let c = self.aviso(n, &itens[0]);
        if itens.len() > 1 {
            let aberto = self.grupos_abertos.borrow().contains(app);
            if aberto {
                for g in &itens[1..] {
                    let l = caixa(Orientation::Vertical, 1, "aviso-anterior");
                    let t = rotulo(&g.titulo, "aviso-titulo");
                    t.set_halign(Align::Start);
                    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    t.set_max_width_chars(1);
                    t.set_hexpand(true);
                    l.append(&t);
                    if !g.corpo.is_empty() {
                        let b = rotulo(&g.corpo, "aviso-texto");
                        b.set_halign(Align::Start);
                        b.set_ellipsize(gtk::pango::EllipsizeMode::End);
                        b.set_max_width_chars(1);
                        b.set_hexpand(true);
                        l.append(&b);
                    }
                    c.append(&l);
                }
            }
            let mais = gtk::Button::new();
            mais.add_css_class("fantasma");
            mais.add_css_class("aviso-mais");
            let mb = caixa(Orientation::Horizontal, 6, "");
            let linha = caixa(Orientation::Horizontal, 0, "aviso-mais-linha");
            linha.set_valign(Align::Center);
            mb.append(&linha);
            let nome = if app.is_empty() { tr("Notice").to_string() } else { app.to_string() };
            mb.append(&rotulo(&if aberto { tr("Show less").to_string() } else { trf("+{n} from {app}", &[("n", (itens.len() - 1).to_string()), ("app", nome)]) }, "aviso-mais-texto"));
            mb.append(&ui_icon(if aberto { "seta-cima" } else { "seta-baixo" }, MUTED, 16));
            mais.set_child(Some(&mb));
            mais.set_halign(Align::Start);
            let (w, app) = (Rc::downgrade(self), app.to_string());
            mais.connect_clicked(move |_| {
                if let Some(d) = w.upgrade() {
                    {
                        let mut g = d.grupos_abertos.borrow_mut();
                        if !g.remove(&app) {
                            g.insert(app.clone());
                        }
                    }
                    d.render_avisos(false);
                }
            });
            c.append(&mais);
        }
        c
    }

    fn aviso(self: &Rc<Self>, n: &Rc<notificacoes::Notificacoes>, g: &notificacoes::Guardado) -> gtk::Box {
        let c = cartao("aviso-central");
        c.set_spacing(10);
        let topo = caixa(Orientation::Horizontal, 8, "");
        if let Some(i) = notificacoes::imagem(&g.icone, 16) {
            topo.append(&i);
        }
        topo.append(&rotulo(if g.app.is_empty() { tr("Notice") } else { &g.app }, "aviso-app"));
        let q = rotulo(&format!("· {}", tempo_relativo(g.quando)), "aviso-hora");
        q.set_hexpand(true);
        q.set_halign(Align::Start);
        topo.append(&q);
        if !g.lida {
            let ponto = caixa(Orientation::Horizontal, 0, "aviso-nao-lida");
            ponto.set_valign(Align::Center);
            ponto.set_tooltip_text(Some(tr("Unread")));
            topo.append(&ponto);
        }
        let x = gtk::Button::new();
        x.add_css_class("aviso-fechar");
        x.set_child(Some(&ui_icon("fechar", MUTED, 16)));
        x.set_tooltip_text(Some(tr("Dismiss")));
        topo.append(&x);
        c.append(&topo);
        let meio = caixa(Orientation::Horizontal, 10, "");
        if let Some(img) = g.imagem.as_deref().and_then(|p| notificacoes::imagem(p, 36)) {
            img.set_valign(Align::Start);
            meio.append(&img);
        }
        let col = caixa(Orientation::Vertical, 2, "");
        let ti = rotulo(&g.titulo, "aviso-titulo");
        ti.set_halign(Align::Start);
        ti.set_wrap(true);
        ti.set_xalign(0.0);
        ti.set_max_width_chars(38);
        col.append(&ti);
        if !g.corpo.is_empty() {
            let co = rotulo(&g.corpo, "aviso-texto");
            co.set_halign(Align::Start);
            co.set_wrap(true);
            co.set_xalign(0.0);
            co.set_max_width_chars(44);
            co.set_lines(3);
            co.set_ellipsize(gtk::pango::EllipsizeMode::End);
            col.append(&co);
        }
        meio.append(&col);
        c.append(&meio);
        let acoes: Vec<_> = g.acoes.iter().filter(|(k, _)| k != "default").cloned().collect();
        if !acoes.is_empty() || !g.lida {
            let linha = caixa(Orientation::Horizontal, 8, "");
            linha.set_homogeneous(true);
            for (i, (k, rot)) in acoes.into_iter().enumerate() {
                let b = gtk::Button::with_label(&rot);
                b.add_css_class("aviso-acao");
                if i == 0 {
                    b.add_css_class("primaria");
                }
                let (nn, id) = (n.clone(), g.id);
                b.connect_clicked(move |_| nn.acionar(id, &k));
                linha.append(&b);
            }
            if !g.lida {
                let b = gtk::Button::with_label(tr("Mark as read"));
                b.add_css_class("aviso-acao");
                let (nn, id) = (n.clone(), g.id);
                b.connect_clicked(move |_| nn.marcar_lida(id));
                linha.append(&b);
            }
            c.append(&linha);
        }
        let (nn, id) = (n.clone(), g.id);
        let me = Rc::downgrade(self);
        x.connect_clicked(move |b| {
            // dispensar desliza para a direita (mola rápida)
            let alvo = b.ancestor(Animavel::static_type()).and_then(|a| a.downcast::<Animavel>().ok());
            let nn = nn.clone();
            match alvo {
                Some(a) => a.sair((60.0, 0.0), 1.0, move || nn.dispensar(id)),
                None => nn.dispensar(id),
            }
            let _ = me.upgrade();
        });
        c
    }

    fn calendario(self: &Rc<Self>) -> gtk::Box {
        // design "Calendário": an inner block (302 px, padding 18) inside the card
        let c = cartao("calendario");
        let dentro = caixa(Orientation::Vertical, 10, "calendario-dentro");
        dentro.set_halign(Align::Center);
        c.append(&dentro);
        let (ano, mes) = self.mes.get();
        let topo = caixa(Orientation::Horizontal, 6, "");
        let primeiro = glib::DateTime::from_local(ano, mes, 1, 0, 0, 0.0).unwrap();
        let titulo = primeiro.format("%B %Y").map(|s| {
            let s = s.to_string();
            let mut ch = s.chars();
            ch.next().map(|f| f.to_uppercase().collect::<String>() + ch.as_str()).unwrap_or_default()
        }).unwrap_or_default();
        let tl = rotulo(&titulo, "cal-titulo");
        tl.set_hexpand(true);
        tl.set_halign(Align::Start);
        topo.append(&tl);
        let seta = |glifo: &str, dica: &str| {
            let b = gtk::Button::new();
            b.set_child(Some(&ui_icon(glifo, MUTED, 16)));
            b.add_css_class("fantasma");
            b.add_css_class("cal-nav");
            b.set_tooltip_text(Some(dica));
            b
        };
        let ant = seta("seta-esquerda", tr("Previous month"));
        let prox = seta("seta-direita", tr("Next month"));
        topo.append(&ant);
        topo.append(&prox);
        dentro.append(&topo);
        let grade = gtk::Grid::new();
        grade.set_column_spacing(4);
        grade.set_row_spacing(0);
        // weekday initials from the system locale, from the first day the person chose (Settings › System › Date:
        // Sunday or Monday), via glib
        let primeiro_dia = crate::configuracoes::ajustes()["primeiro_dia_da_semana"].as_i64().unwrap_or(0).clamp(0, 6) as i32;
        let domingo = glib::DateTime::from_local(2023, 1, 1, 12, 0, 0.0).unwrap();
        for i in 0..7 {
            let d = domingo.add_days(i + primeiro_dia).ok().and_then(|d| d.format("%a").ok()).map(|s| s.chars().next().unwrap_or(' ').to_uppercase().to_string()).unwrap_or_default();
            let l = rotulo(&d, "cal-semana");
            l.set_size_request(34, 24);
            grade.attach(&l, i, 0, 1, 1);
        }
        let hoje = glib::DateTime::now_local().unwrap();
        let inicio = (primeiro.day_of_week() % 7 - primeiro_dia + 7) % 7; // columns from the chosen first day
        let dias = primeiro.add_months(1).and_then(|d| d.add_days(-1)).map(|d| d.day_of_month()).unwrap_or(30);
        let dias_antes = primeiro.add_days(-1).map(|d| d.day_of_month()).unwrap_or(30);
        // whole weeks: the end of the month before and the start of the next, dimmed
        let semanas = (inicio + dias + 6) / 7;
        for pos in 0..semanas * 7 {
            let (texto, fora) = if pos < inicio {
                (dias_antes - inicio + pos + 1, true)
            } else if pos - inicio < dias {
                (pos - inicio + 1, false)
            } else {
                (pos - inicio - dias + 1, true)
            };
            let l = rotulo(&texto.to_string(), "cal-dia");
            l.set_size_request(34, 34);
            if fora {
                l.add_css_class("fora");
            } else if ano == hoje.year() && mes == hoje.month() && texto == hoje.day_of_month() {
                l.add_css_class("hoje");
            }
            l.set_halign(Align::Center);
            l.set_valign(Align::Center);
            grade.attach(&l, pos % 7, 1 + pos / 7, 1, 1);
        }
        let grade_a = Animavel::new(&grade);
        dentro.append(&grade_a);
        // trocar de mês: a grade desliza (mola lenta, percurso de página)
        for (b, passo) in [(ant, -1), (prox, 1)] {
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(d) = w.upgrade() {
                    let (a, m) = d.mes.get();
                    let t = a * 12 + (m - 1) + passo;
                    d.mes.set((t.div_euclid(12), t.rem_euclid(12) + 1));
                    d.render_base();
                    // a grade nova chega do lado do passo
                    glib::idle_add_local_once({
                        let w = Rc::downgrade(&d);
                        move || {
                            if let Some(d) = w.upgrade() {
                                d.deslizar_calendario(passo as f64 * 40.0);
                            }
                        }
                    });
                }
            });
        }
        c
    }

    fn deslizar_calendario(&self, dx: f64) {
        let mut pilha = vec![self.avisos_rodape.clone().upcast::<gtk::Widget>()];
        while let Some(w) = pilha.pop() {
            if w.has_css_class("calendario-dentro") {
                if let Some(g) = w.last_child().and_then(|g| g.downcast::<Animavel>().ok()) {
                    g.definir(Canal::X, dx).definir(Canal::Opacidade, 0.3);
                    g.ir(Canal::X, 0.0, mv::LENTA).ir(Canal::Opacidade, 1.0, mv::LENTA);
                }
                return;
            }
            let mut c = w.first_child();
            while let Some(x) = c {
                c = x.next_sibling();
                pilha.push(x);
            }
        }
    }

    // ------------------------------------------------------------------------------------------- Controles

    fn ler_sistema(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        em_segundo_plano(|| {
            let s = sistema::ler();
            // what left this computer today: web pages read by agents (the models are local)
            let since = glib::DateTime::now_local().ok().and_then(|n| glib::DateTime::from_local(n.year(), n.month(), n.day_of_month(), 0, 0, 0.0).ok()).map(|d| d.to_unix()).unwrap_or(0);
            let web = servicos::kalangod_linha_do_tempo(200).map(|l| l.iter().filter(|e| e["tool"].as_str().is_some_and(|t| t.starts_with("web.")) && e["time"].as_i64().unwrap_or(0) >= since).count()).unwrap_or(0);
            (s, web)
        }, move |(s, web)| {
            if let Some(d) = w.upgrade() {
                // rebuilt only when something changed: a rebuild under the pointer would swallow a click
                let changed = *d.sistema.borrow() != s || d.web_hoje.get() != web;
                d.web_hoje.set(web);
                *d.sistema.borrow_mut() = s;
                if changed && d.painel.aberta.get() {
                    match d.modo.get() {
                        Modo::Controles => d.render_controles(false),
                        Modo::Avisos => d.render_base(),
                        Modo::Agente => {}
                    }
                }
                if let Some(sh) = d.shell() {
                    sh.estado_sistema(&d.sistema.borrow());
                }
            }
        });
    }

    /// Roda uma ação do sistema em segundo plano e relê o estado.
    fn agir(self: &Rc<Self>, f: impl FnOnce() + Send + 'static) {
        let w = Rc::downgrade(self);
        em_segundo_plano(f, move |_| {
            if let Some(d) = w.upgrade() {
                d.ler_sistema();
            }
        });
    }

    fn bloco(self: &Rc<Self>, glifo: &'static str, titulo: &str, sub: &str, ligado: bool, f: impl Fn(&Rc<Direita>, bool) + 'static) -> gtk::Button {
        let b = gtk::Button::new();
        b.add_css_class("bloco");
        b.set_widget_name(titulo);
        if ligado {
            b.add_css_class("ligado");
        }
        let linha = caixa(Orientation::Horizontal, 10, "");
        linha.set_valign(Align::Center);
        let ic = ui_icon(glifo, if ligado { ON_INK } else { TEXT }, 16);
        linha.append(&ic);
        let col = caixa(Orientation::Vertical, 0, "");
        col.set_valign(Align::Center);
        let t = rotulo(titulo, "bloco-titulo");
        t.set_halign(Align::Start);
        let s = rotulo(sub, "bloco-sub");
        s.set_halign(Align::Start);
        s.set_ellipsize(gtk::pango::EllipsizeMode::End);
        s.set_max_width_chars(16);
        col.append(&t);
        col.append(&s);
        linha.append(&col);
        let a = Animavel::new(&linha);
        b.set_child(Some(&a));
        b.set_hexpand(true);
        let w = Rc::downgrade(self);
        b.connect_clicked(move |b| {
            let novo = !b.has_css_class("ligado");
            if novo { b.add_css_class("ligado") } else { b.remove_css_class("ligado") }
            crate::conversa::set_icon(&ic, glifo, if novo { ON_INK } else { TEXT });
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug controles: clique em {} → {novo}", b.widget_name());
            }
            // resposta na hora: o bloco "assenta" com a mola elástica
            a.definir(Canal::Escala, 0.94);
            a.ir(Canal::Escala, 1.0, mv::ELASTICA);
            if let Some(d) = w.upgrade() {
                f(&d, novo);
            }
        });
        b
    }

    fn render_controles(self: &Rc<Self>, cascata: bool) {
        let Some(shell) = self.shell() else { return };
        let s = self.sistema.borrow().clone();
        let mut blocos: Vec<gtk::Widget> = Vec::new();

        // pessoa, bateria e atalhos
        let topo = caixa(Orientation::Horizontal, 10, "");
        let nome = crate::energia::person_name();
        topo.append(&crate::energia::avatar(&nome, 36, "avatar-mini"));
        let col = caixa(Orientation::Vertical, 0, "");
        col.set_hexpand(true);
        col.set_valign(Align::Center);
        let n = rotulo(&nome, "pessoa-nome");
        n.set_halign(Align::Start);
        col.append(&n);
        let sub = match s.bateria {
            Some((p, true)) => trf("{p}% · charging", &[("p", p.to_string())]),
            Some((p, false)) => trf("{p}% battery", &[("p", p.to_string())]),
            None => if s.rede.cabo { tr("plugged in · wired network").to_string() } else { tr("plugged in").to_string() },
        };
        let sl = rotulo(&sub, "bloco-sub");
        sl.set_halign(Align::Start);
        col.append(&sl);
        topo.append(&col);
        let cfg = botao_glifo("engrenagem", TEXT, tr("Settings"), "");
        cfg.connect_clicked(|_| abrir_configuracoes());
        let bloq = botao_glifo("cadeado", TEXT, tr("Lock"), "");
        let en = botao_glifo("desligar", CORAL, tr("Power"), "perigo");
        {
            let sh = Rc::downgrade(&shell);
            bloq.connect_clicked(move |_| {
                if let Some(s) = sh.upgrade() {
                    s.energia().executar("bloquear");
                }
            });
            let sh = Rc::downgrade(&shell);
            let me = Rc::downgrade(self);
            en.connect_clicked(move |_| {
                if let (Some(s), Some(d)) = (sh.upgrade(), me.upgrade()) {
                    d.close();
                    s.energia().alternar();
                }
            });
        }
        topo.append(&cfg);
        topo.append(&bloq);
        topo.append(&en);
        blocos.push(topo.upcast());

        // alternâncias (só o que esta máquina oferece)
        let grade = gtk::Grid::new();
        grade.set_column_spacing(8);
        grade.set_row_spacing(8);
        grade.set_column_homogeneous(true);
        let mut itens: Vec<gtk::Button> = Vec::new();
        let wifi_sub = match (&s.rede.conectada, s.rede.wifi_ligado) {
            (Some(n), _) => n.clone(),
            (None, true) => tr("on").into(),
            _ => tr("off").into(),
        };
        itens.push(self.bloco("wifi", tr("Wi-Fi"), &wifi_sub, s.rede.wifi_ligado, |d, v| d.agir(move || sistema::wifi(v))));
        if s.bt.existe {
            let bt_sub = s.bt.conectados.first().map(|x| x.nome.clone()).unwrap_or_else(|| if s.bt.ligado { tr("on").into() } else { tr("off").into() });
            itens.push(self.bloco("bluetooth", tr("Bluetooth"), &bt_sub, s.bt.ligado, |d, v| d.agir(move || sistema::bluetooth(v))));
        }
        let np = shell.notificacoes().nao_perturbe.get();
        itens.push(self.bloco("sino-mudo", tr("Do not disturb"), if np { tr("on") } else { tr("off") }, np, |d, v| {
            if let Some(s) = d.shell() {
                s.notificacoes().definir_nao_perturbe(v);
            }
        }));
        // night light: warm colors until 06:00 (the compositor's gamma, via wlsunset)
        if s.luz_noturna.is_some() && s.luz_noturna_indisponivel {
            let b = self.bloco("lua", tr("Night light"), tr("not available on this screen"), false, |_, _| {});
            b.set_sensitive(false);
            b.set_tooltip_text(Some(tr("The compositor offers no gamma control on this screen")));
            itens.push(b);
        } else if let Some(on) = s.luz_noturna {
            itens.push(self.bloco("lua", tr("Night light"), if on { tr("until 06:00") } else { tr("off") }, on, |d, v| {
                d.agir(move || sistema::luz_noturna(v));
                if v {
                    let w = Rc::downgrade(d);
                    glib::timeout_add_seconds_local_once(sistema::until_morning() as u32, move || {
                        if let Some(d) = w.upgrade() {
                            d.agir(|| sistema::luz_noturna(false));
                        }
                    });
                }
            }));
        }
        itens.push(self.bloco("aviao", tr("Airplane mode"), if s.rede.aviao { tr("on") } else { tr("off") }, s.rede.aviao, |d, v| {
            crate::osd::modo_aviao(v);
            d.agir(move || sistema::aviao(v))
        }));
        // pause agents: they freeze where they are (and go on from there when resumed)
        let (ativos, pausados) = s.agentes;
        let ag_sub = match (ativos, pausados) {
            (0, _) => tr("none running").to_string(),
            (n, p) if p > 0 => trf("{n} paused", &[("n", p.max(n).to_string())]),
            (1, _) => tr("1 running").to_string(),
            (n, _) => trf("{n} running", &[("n", n.to_string())]),
        };
        itens.push(self.bloco("escudo", tr("Pause agents"), &ag_sub, pausados > 0, |d, v| d.agir(move || sistema::pausar_agentes(v))));
        for (i, b) in itens.iter().enumerate() {
            grade.attach(b, (i % 2) as i32, (i / 2) as i32, 1, 1);
        }
        blocos.push(grade.upcast());

        // volume e brilho
        if s.som.existe || s.brilho.is_some() {
            let c = cartao("");
            if s.som.existe {
                let nivel = if s.som.mudo { "volume-mudo" } else if s.som.nivel > 0.33 { "volume" } else { "volume-baixo" };
                c.append(&self.deslizador(nivel, s.som.nivel, "acento", |d, v| d.agir(move || sistema::volume(v)), Some(Box::new({
                    let mudo = s.som.mudo;
                    move |d: &Rc<Direita>| d.agir(move || sistema::mudo(!mudo))
                }))));
            }
            if let Some(b) = s.brilho {
                c.append(&self.deslizador("sol", b, "ouro", |d, v| d.agir(move || sistema::brilho(v)), None));
            }
            blocos.push(c.upcast());
        }

        // modo de energia
        if !s.perfil.is_empty() {
            let c = cartao("");
            let cab = caixa(Orientation::Horizontal, 6, "");
            cab.append(&ui_icon("raio", MUTED, 16));
            cab.append(&secao(tr("POWER MODE")));
            c.append(&cab);
            let seg = caixa(Orientation::Horizontal, 0, "segmentos");
            let mut primeiro: Option<gtk::ToggleButton> = None;
            for (id, rot) in [("power-saver", tr("Power saver")), ("balanced", tr("Balanced")), ("performance", tr("Performance"))] {
                if !s.perfis.iter().any(|p| p == id) {
                    continue;
                }
                let b = gtk::ToggleButton::with_label(rot);
                b.set_hexpand(true);
                if let Some(p) = &primeiro {
                    b.set_group(Some(p));
                } else {
                    primeiro = Some(b.clone());
                }
                b.set_active(s.perfil == id);
                let w = Rc::downgrade(self);
                b.connect_toggled(move |b| {
                    if b.is_active() {
                        if let Some(d) = w.upgrade() {
                            if !d.atualizando.get() && d.sistema.borrow().perfil != id {
                                d.agir(move || sistema::perfil(id));
                            }
                        }
                    }
                });
                seg.append(&b);
            }
            c.append(&seg);
            blocos.push(c.upcast());
        }

        // mídia
        if let Some(m) = shell.midia.atual() {
            blocos.push(cartao_midia(&shell, &m).upcast());
        }

        // dispositivos: Bluetooth conectados e monitores (pelo niri)
        let disp = cartao("lista");
        for dv in &s.bt.conectados {
            let ic = match dv.tipo.as_str() {
                t if t.contains("headset") || t.contains("audio") => "fone",
                t if t.contains("phone") => "celular",
                _ => "bluetooth",
            };
            disp.append(&linha_glifo(ic, &dv.nome, tr("connected"), dv.bateria.map(|b| format!("{b}%")).as_deref(), false));
        }
        if let Ok(saidas) = servicos::niri_pedido(serde_json::json!("Outputs")) {
            if let Some(o) = saidas.get("Outputs").and_then(Value::as_object) {
                for (nome, v) in o {
                    let modo = v["modes"].get(v["current_mode"].as_u64().unwrap_or(0) as usize);
                    let res = modo.map(|m| format!("{} × {} · {:.0} Hz", m["width"], m["height"], m["refresh_rate"].as_f64().unwrap_or(0.0) / 1000.0)).unwrap_or_default();
                    let titulo = v["model"].as_str().filter(|s| !s.is_empty()).unwrap_or(nome).to_string();
                    let linha = linha_glifo("monitor", &titulo, &res, None, true);
                    let clique = gtk::GestureClick::new();
                    clique.connect_released(|_, _, _, _| abrir_painel("display"));
                    linha.add_controller(clique);
                    disp.append(&linha);
                }
            }
        }
        if disp.first_child().is_some() {
            blocos.push(disp.upcast());
        }
        // footer at the bottom of the bar: what left this computer today, and all settings
        let rodape = caixa(Orientation::Horizontal, 8, "rodape-lateral");
        let web = self.web_hoje.get();
        rodape.append(&ui_icon("escudo", if web == 0 { TEAL } else { MUTED }, 16));
        let privacidade = rotulo(&if web == 0 { tr("nothing sent to the cloud today").to_string() } else {
            trf("only web pages read by agents left today ({n})", &[("n", web.to_string())])
        }, "rodape-texto");
        privacidade.set_hexpand(true);
        privacidade.set_xalign(0.0);
        privacidade.set_ellipsize(gtk::pango::EllipsizeMode::End);
        rodape.append(&privacidade);
        let todas = gtk::Button::new();
        let tb = caixa(Orientation::Horizontal, 6, "");
        tb.append(&ui_icon("engrenagem", MUTED, 16));
        tb.append(&rotulo(tr("All settings"), "rodape-botao"));
        todas.set_child(Some(&tb));
        todas.add_css_class("fantasma");
        todas.connect_clicked(|_| abrir_configuracoes());
        rodape.append(&todas);
        limpar(&self.controles_rodape);
        self.controles_rodape.append(&rodape);
        // test aid: where the tiles and the pin are on the output (the bar sits at the right edge with an 8 px margin)
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() && cascata {
            // the tiles are rebuilt while the bar is open: look them up when printing
            let (pagina, alfinete, janela) = (self.controles.clone(), self.alfinete.clone(), self.painel.janela.clone());
            glib::timeout_add_local_once(Duration::from_millis(900), move || {
                let out_w = gdk::Display::default().and_then(|d| d.monitors().item(0)).and_downcast::<gdk::Monitor>().map(|m| m.geometry().width()).unwrap_or(1920);
                let dx = f64::from(out_w - 8 - janela.width());
                let dy = f64::from(janela.margin(Edge::Top) + if janela.exclusive_zone() == -1 { 0 } else { BARRA_ALTURA });
                fn tiles(w: &gtk::Widget, out: &mut Vec<gtk::Widget>) {
                    if w.has_css_class("bloco") {
                        out.push(w.clone());
                    }
                    let mut c = w.first_child();
                    while let Some(x) = c {
                        tiles(&x, out);
                        c = x.next_sibling();
                    }
                }
                let mut found = Vec::new();
                tiles(pagina.upcast_ref(), &mut found);
                for w in found {
                    if let Some(b) = w.compute_bounds(&janela) {
                        eprintln!("debug controles: {} em {} {}", w.widget_name(), (dx + f64::from(b.x() + b.width() / 2.)) as i32, (dy + f64::from(b.y() + b.height() / 2.)) as i32);
                    }
                }
                if let Some(b) = alfinete.compute_bounds(&janela) {
                    eprintln!("debug controles: fixar em {} {}", (dx + f64::from(b.x() + b.width() / 2.)) as i32, (dy + f64::from(b.y() + b.height() / 2.)) as i32);
                }
            });
        }
        self.atualizando.set(true);
        self.montar(&self.controles, blocos, cascata);
        self.atualizando.set(false);
    }

    fn deslizador(self: &Rc<Self>, icone_nome: &str, valor: f64, cor: &str, f: impl Fn(&Rc<Direita>, f64) + 'static, clique_icone: Option<IconAction>) -> gtk::Box {
        let l = caixa(Orientation::Horizontal, 10, "deslizador");
        let ib = gtk::Button::new();
        ib.add_css_class("fantasma");
        ib.add_css_class("deslizador-icone");
        ib.set_child(Some(&ui_icon(icone_nome, TEXT, 16)));
        if let Some(c) = clique_icone {
            let w = Rc::downgrade(self);
            ib.connect_clicked(move |_| {
                if let Some(d) = w.upgrade() {
                    c(&d);
                }
            });
        }
        l.append(&ib);
        let esc = gtk::Scale::with_range(Orientation::Horizontal, 0.0, 1.0, 0.01);
        esc.set_draw_value(false);
        esc.set_value(valor);
        esc.set_hexpand(true);
        esc.add_css_class(cor);
        let pct = rotulo(&format!("{:.0}%", valor * 100.0), "pct");
        let p2 = pct.clone();
        // aplica ao soltar/parar de arrastar (sem inundar o sistema de comandos)
        let pendente: Rc<Cell<Option<glib::SourceId>>> = Rc::default();
        let w = Rc::downgrade(self);
        let f = Rc::new(f);
        esc.connect_value_changed(move |e| {
            p2.set_text(&format!("{:.0}%", e.value() * 100.0));
            if let Some(id) = pendente.take() {
                id.remove();
            }
            let (w, f, v, pend) = (w.clone(), f.clone(), e.value(), pendente.clone());
            pendente.set(Some(glib::timeout_add_local_once(Duration::from_millis(120), move || {
                pend.take();
                if let Some(d) = w.upgrade() {
                    if !d.atualizando.get() {
                        f(&d, v);
                    }
                }
            })));
        });
        l.append(&esc);
        l.append(&pct);
        l
    }

    // ------------------------------------------------------------------------------------------- Agente

    fn render_agente(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        em_segundo_plano(|| (servicos::kalangod_pendentes(), servicos::kalangod_linha_do_tempo(60)), move |(pend, linha)| {
            let Some(d) = w.upgrade() else { return };
            // under the conversation greeting: what waits for you and what was done
            let mut blocos: Vec<gtk::Widget> = Vec::new();
            match (&pend, &linha) {
                (Err(e), _) => {
                    let c = cartao("vazio-lateral");
                    c.append(&rotulo(tr("kalangod is not responding."), "bloco-titulo"));
                    let l = rotulo(e, "fraco");
                    l.set_wrap(true);
                    c.append(&l);
                    blocos.push(c.upcast());
                }
                (Ok(p), _) => {
                    if !p.is_empty() {
                        blocos.push(secao(tr("WAITING FOR YOU")).upcast());
                        for x in p {
                            let c = cartao("pedido");
                            c.append(&linha_info("dialog-warning-symbolic", x["tool"].as_str().unwrap_or(""),
                                &trf("{caller} · level {level}", &[("caller", x["caller"].as_str().unwrap_or("").split(" (").next().unwrap_or("").to_string()), ("level", x["level"].as_str().unwrap_or("").to_string())]), None));
                            let dica = rotulo(tr("Decide on the approval screen."), "fraco");
                            dica.set_halign(Align::Start);
                            c.append(&dica);
                            blocos.push(c.upcast());
                        }
                    }
                }
            }
            if let Ok(l) = &linha {
                blocos.push(secao(tr("TIMELINE")).upcast());
                let c = cartao("lista");
                if l.is_empty() {
                    c.append(&rotulo(tr("No actions recorded yet."), "fraco"));
                }
                // (the same tool in a row is one line: "Pesquisou na web · 9 buscas", open to see each)
                for g in group_timeline(l).iter().take(12) {
                    c.append(&linha_do_tempo(&l[g.first..g.first + g.len]));
                }
                blocos.push(c.upcast());
            }
            d.montar(&d.agente, blocos, true);
        });
    }
}

/// Consecutive entries of the timeline of one tool and one outcome, shown as one line. `first` is the index of its
/// first entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimelineGroup {
    pub tool: String,
    pub first: usize,
    pub len: usize,
}

/// The lines of the timeline (entries newest first, as kalangod gives them): the same tool, one after another and
/// ending the same way, is one line ("Pesquisou na web · 9 buscas"). A failure, a denial or an undone action does
/// not hide among the ones that went well: it starts its own line.
pub fn group_timeline(entries: &[Value]) -> Vec<TimelineGroup> {
    let mut out: Vec<TimelineGroup> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let tool = e["tool"].as_str().unwrap_or("");
        let status = e["status"].as_str().unwrap_or("");
        let joins = out.last().is_some_and(|g| g.tool == tool && !tool.is_empty() && entries[g.first + g.len - 1]["status"].as_str().unwrap_or("") == status);
        match out.last_mut() {
            Some(g) if joins => g.len += 1,
            _ => out.push(TimelineGroup { tool: tool.to_string(), first: i, len: 1 }),
        }
    }
    out
}

/// A line's title and count chip for `n` entries of a tool: one entry is its own summary; many that went well are what
/// the tool did ("Leu 4 arquivos"), with a chip where the design counts them ("9 buscas"); many that failed or were
/// denied are the tool and their count.
pub fn timeline_words(tool: &str, n: usize, summary: &str, done: bool) -> (String, Option<String>) {
    if n <= 1 {
        return (if summary.is_empty() { tool.to_string() } else { summary.to_string() }, None);
    }
    if !done {
        return (tool.to_string(), Some(n.to_string()));
    }
    // (the words of the chat's steps, chat_blocks.rs: one place says what each tool did)
    let label = crate::chat_blocks::step_label(tool, n, crate::chat_blocks::StepState::Done);
    if label == tool {
        return (tool.to_string(), Some(n.to_string()));
    }
    (label, crate::chat_blocks::step_chip(tool, n, false))
}

/// One line of the timeline: a group of entries (one, or many of the same tool, open to list them).
fn linha_do_tempo(itens: &[Value]) -> gtk::Widget {
    let e = &itens[0];
    let st = e["status"].as_str().unwrap_or("");
    let ic = match st {
        "ok" | "simulado" => "emblem-ok-symbolic",
        "desfeito" => "edit-undo-symbolic",
        "negado" | "bloqueado" => "action-unavailable-symbolic",
        "erro" => "dialog-error-symbolic",
        _ => "view-list-symbolic",
    };
    let tool = e["tool"].as_str().unwrap_or("");
    let quando = e["time"].as_i64().map(tempo_relativo).unwrap_or_default();
    let (titulo, chip) = timeline_words(tool, itens.len(), e["summary"].as_str().unwrap_or(""), matches!(st, "ok" | "simulado"));
    let linha = linha_info(ic, &titulo, &format!("{tool} · {st} · {quando}"), None);
    if itens.len() == 1 {
        return linha.upcast();
    }
    // the count beside the title, and a chevron that turns when the list opens
    if let Some(col) = linha.first_child().and_then(|i| i.next_sibling()).and_downcast::<gtk::Box>() {
        if let Some(t) = col.first_child() {
            let topo = caixa(Orientation::Horizontal, 8, "");
            col.remove(&t);
            topo.append(&t);
            if let Some(c) = &chip {
                let l = rotulo(c, "linha-tempo-chip");
                l.set_valign(Align::Center);
                topo.append(&l);
            }
            col.prepend(&topo);
        }
    }
    let seta = ui_icon("seta-direita", MUTED, 14);
    seta.set_valign(Align::Center);
    linha.append(&seta);
    let lista = caixa(Orientation::Vertical, 2, "linha-tempo-itens");
    for x in itens {
        let r = caixa(Orientation::Horizontal, 8, "");
        let t = rotulo(x["summary"].as_str().unwrap_or(tool), "linha-tempo-item");
        t.set_halign(Align::Start);
        t.set_hexpand(true);
        t.set_ellipsize(gtk::pango::EllipsizeMode::End);
        t.set_max_width_chars(34);
        r.append(&t);
        r.append(&rotulo(&x["time"].as_i64().map(tempo_relativo).unwrap_or_default(), "fraco"));
        lista.append(&r);
    }
    let rv = gtk::Revealer::new();
    rv.set_transition_type(gtk::RevealerTransitionType::SlideDown);
    rv.set_transition_duration(220);
    rv.set_child(Some(&lista));
    let b = gtk::Button::new();
    b.add_css_class("linha-tempo-grupo");
    b.set_child(Some(&linha));
    let r2 = rv.clone();
    b.connect_clicked(move |_| {
        let abrir = !r2.reveals_child();
        r2.set_transition_duration(if abrir { 220 } else { 160 });
        r2.set_reveal_child(abrir);
        // (the chevron turns 90° with the list)
        crate::conversa::set_icon(&seta, if abrir { "seta-baixo" } else { "seta-direita" }, MUTED);
    });
    let caixa_g = caixa(Orientation::Vertical, 0, "");
    caixa_g.append(&b);
    caixa_g.append(&rv);
    caixa_g.upcast()
}

/// The summary card shows from this many unread notices.
const RESUMO_MINIMO: usize = 4;
const ACCENT: &str = "#A5A1E2";

/// The three points from the local model (`kalango-agente avisos`); without it, the latest titles.
fn resumir_avisos(dados: &serde_json::Value) -> Vec<String> {
    use std::io::Write;
    let filho = std::process::Command::new(crate::voz::agent_binary()).arg("avisos")
        .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn();
    let pontos = filho.ok().and_then(|mut c| {
        let _ = c.stdin.take()?.write_all(dados.to_string().as_bytes());
        let o = c.wait_with_output().ok()?;
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).ok()?;
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug avisos: resumo {} ({} pontos)", if v["gerado"] == true { "do modelo local" } else { "sem o modelo" }, v["pontos"].as_array().map(|a| a.len()).unwrap_or(0));
        }
        Some(v["pontos"].as_array()?.iter().filter_map(|p| p.as_str().map(str::to_string)).collect::<Vec<_>>())
    });
    pontos.filter(|p| !p.is_empty()).unwrap_or_else(|| dados.as_array().into_iter().flatten().take(3)
        .map(|n| format!("{}: {}", n["app"].as_str().unwrap_or(""), n["titulo"].as_str().unwrap_or(""))).collect())
}

const TEXT: &str = "#E2E2E6";
const MUTED: &str = "#8E8F99";
const ON_INK: &str = "#1C1D24";
const CORAL: &str = "#EFA2A6";
const TEAL: &str = "#8ED2C3";
const GOLD: &str = "#E5C98C";

/// A square icon button with one of the design's glyphs.
fn botao_glifo(glifo: &str, cor: &str, dica: &str, classe: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.set_child(Some(&ui_icon(glifo, cor, 16)));
    b.add_css_class("botao-icone");
    if !classe.is_empty() {
        b.add_css_class(classe);
    }
    b.set_tooltip_text(Some(dica));
    b
}

/// A device row (design "Dispositivos"): glyph, name, detail, and a value or a chevron at the end.
fn linha_glifo(glifo: &str, titulo: &str, sub: &str, fim: Option<&str>, seta: bool) -> gtk::Box {
    let l = caixa(Orientation::Horizontal, 12, "linha-dispositivo");
    l.append(&ui_icon(glifo, TEXT, 16));
    let col = caixa(Orientation::Vertical, 1, "");
    col.set_hexpand(true);
    col.set_valign(Align::Center);
    let t = rotulo(titulo, "dispositivo-nome");
    t.set_halign(Align::Start);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    t.set_max_width_chars(30);
    col.append(&t);
    if !sub.is_empty() {
        let s = rotulo(sub, "dispositivo-sub");
        s.set_halign(Align::Start);
        s.set_ellipsize(gtk::pango::EllipsizeMode::End);
        s.set_max_width_chars(36);
        col.append(&s);
    }
    l.append(&col);
    if let Some(f) = fim {
        l.append(&rotulo(f, "dispositivo-valor"));
    }
    if seta {
        l.append(&ui_icon("seta-direita", MUTED, 16));
    }
    l
}

fn abrir_painel(painel: &str) {
    crate::configuracoes::abrir_painel(painel);
}

fn linha_info(icone_nome: &str, titulo: &str, sub: &str, fim: Option<&str>) -> gtk::Box {
    let l = caixa(Orientation::Horizontal, 10, "linha-info");
    let i = icone(icone_nome, 16);
    i.set_valign(Align::Center);
    l.append(&i);
    let col = caixa(Orientation::Vertical, 0, "");
    col.set_hexpand(true);
    let t = rotulo(titulo, "bloco-titulo");
    t.set_halign(Align::Start);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    t.set_max_width_chars(34);
    col.append(&t);
    if !sub.is_empty() {
        let s = rotulo(sub, "bloco-sub");
        s.set_halign(Align::Start);
        s.set_ellipsize(gtk::pango::EllipsizeMode::End);
        s.set_max_width_chars(40);
        col.append(&s);
    }
    l.append(&col);
    if let Some(f) = fim {
        l.append(&rotulo(f, "fraco"));
    }
    l
}

pub fn cartao_midia(shell: &Rc<Shell>, m: &Midia) -> gtk::Box {
    let c = cartao("midia");
    let l = caixa(Orientation::Horizontal, 12, "");
    l.append(&midia::capa(m, 44));
    let col = caixa(Orientation::Vertical, 0, "");
    col.set_hexpand(true);
    col.set_valign(Align::Center);
    let t = rotulo(&m.titulo, "bloco-titulo");
    t.set_halign(Align::Start);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    t.set_max_width_chars(22);
    let a = rotulo(&m.artista, "bloco-sub");
    a.set_halign(Align::Start);
    a.set_ellipsize(gtk::pango::EllipsizeMode::End);
    a.set_max_width_chars(26);
    col.append(&t);
    col.append(&a);
    l.append(&col);
    for (ic, cmd) in [("media-skip-backward-symbolic", "Previous"), (if m.tocando { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" }, "PlayPause"), ("media-skip-forward-symbolic", "Next")] {
        let b = botao_icone(ic, "", "fantasma");
        let s = Rc::downgrade(shell);
        b.connect_clicked(move |_| {
            if let Some(s) = s.upgrade() {
                s.midia.comando(cmd);
            }
        });
        l.append(&b);
    }
    c.append(&l);
    c
}

fn abrir_configuracoes() {
    if let Some(s) = crate::shell_atual() {
        s.abrir_configuracoes(None);
    }
}

// ================================================================================================= barra esquerda: Hoje

/// The widgets of the "Hoje" bar, in order (the person can hide and bring back each one).
const WIDGETS: &[(&str, &str)] = &[
    ("resumo", n_("Day summary")),
    ("clima", n_("Weather")),
    ("foco", n_("Focus")),
    ("agenda", n_("Today's agenda")),
    ("cpu", n_("CPU")),
    ("gpu", n_("GPU")),
    ("midia", n_("Now playing")),
];

fn hoje_config() -> std::path::PathBuf {
    glib::user_config_dir().join("kalango/hoje.json")
}

fn hidden_widgets() -> Vec<String> {
    std::fs::read_to_string(hoje_config()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["ocultos"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())).unwrap_or_default()
}

fn save_hidden(h: &[String]) {
    let f = hoje_config();
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&f, serde_json::to_string_pretty(&serde_json::json!({"ocultos": h})).unwrap_or_default());
}

pub struct Hoje {
    painel: Painel,
    corpo: gtk::Box,
    rodape: gtk::Box,
    alfinete: gtk::Button,
    fixada: Cell<bool>,
    shell: RefCell<Weak<Shell>>,
    cpu: RefCell<sistema::Cpu>,
    cpu_uso: Cell<f64>,
    gpu: RefCell<Option<(u8, u8, String)>>,
    // foco (cronômetro)
    foco_total: Cell<u32>,
    foco_restante: Cell<u32>,
    foco_rodando: Cell<bool>,
    /// focus sessions finished today (of 4)
    sessoes: Cell<u32>,
    editando: Cell<bool>,
    /// the weather, when the person picked a city: (city, forecast, when it was read)
    clima: RefCell<Option<(clima::City, clima::Weather, i64)>>,
}

impl Hoje {
    pub fn new(app: &gtk::Application) -> Rc<Hoje> {
        let raiz = caixa(Orientation::Vertical, 12, "lateral");
        raiz.add_css_class("hoje");
        let topo = caixa(Orientation::Horizontal, 6, "");
        let col = caixa(Orientation::Vertical, 0, "");
        col.set_hexpand(true);
        let t = rotulo(tr("Today"), "hoje-titulo");
        t.set_halign(Align::Start);
        let data = glib::DateTime::now_local().unwrap().format("%A, %-d de %B").map(|s| s.to_string()).unwrap_or_default();
        let d = rotulo(&data, "hoje-data");
        d.set_halign(Align::Start);
        col.append(&t);
        col.append(&d);
        topo.append(&col);
        let alfinete = gtk::Button::new();
        alfinete.set_child(Some(&ui_icon("alfinete", MUTED, 28)));
        alfinete.add_css_class("fantasma");
        alfinete.add_css_class("alfinete");
        alfinete.set_tooltip_text(Some(tr("Pin: keep the bar open beside the windows")));
        topo.append(&alfinete);
        let fechar = gtk::Button::new();
        fechar.set_child(Some(&ui_icon("fechar", MUTED, 16)));
        fechar.add_css_class("fantasma");
        fechar.set_tooltip_text(Some(tr("Close (Esc)")));
        topo.append(&fechar);
        raiz.append(&topo);
        let corpo = caixa(Orientation::Vertical, 12, "");
        let r = gtk::ScrolledWindow::new();
        r.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        r.set_vexpand(true);
        r.set_child(Some(&corpo));
        raiz.append(&r);
        let rodape = caixa(Orientation::Horizontal, 8, "");
        raiz.append(&rodape);
        let painel = Painel::new(app, "kalango-barra-lateral-esquerda", false, &raiz, false);
        let h = Rc::new(Hoje {
            painel, corpo, rodape, alfinete: alfinete.clone(), fixada: Cell::new(false), shell: RefCell::default(),
            cpu: RefCell::new(sistema::Cpu::ler()), cpu_uso: Cell::new(0.0), gpu: RefCell::default(),
            foco_total: Cell::new(25 * 60), foco_restante: Cell::new(25 * 60), foco_rodando: Cell::new(false), sessoes: Cell::new(0),
            editando: Cell::new(false), clima: RefCell::default(),
        });
        let w = Rc::downgrade(&h);
        alfinete.connect_clicked(move |_| {
            if let Some(h) = w.upgrade() {
                h.set_pinned(!h.fixada.get());
            }
        });
        let w = Rc::downgrade(&h);
        fechar.connect_clicked(move |_| {
            if let Some(h) = w.upgrade() {
                h.close();
            }
        });
        let teclas = gtk::EventControllerKey::new();
        let w = Rc::downgrade(&h);
        teclas.connect_key_pressed(move |_, k, _, _| {
            if k == gdk::Key::Escape {
                if let Some(h) = w.upgrade() {
                    h.close();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        h.painel.janela.add_controller(teclas);
        // a cada 1,5 s: CPU e GPU (só com a barra aberta); a cada 1 s: o foco
        let w = Rc::downgrade(&h);
        glib::timeout_add_local(Duration::from_millis(1500), move || {
            let Some(h) = w.upgrade() else { return glib::ControlFlow::Break };
            if h.painel.aberta.get() {
                let agora = sistema::Cpu::ler();
                h.cpu_uso.set(agora.uso_desde(&h.cpu.borrow()));
                *h.cpu.borrow_mut() = agora;
                let w2 = Rc::downgrade(&h);
                em_segundo_plano(sistema::gpu, move |g| {
                    if let Some(h) = w2.upgrade() {
                        *h.gpu.borrow_mut() = g;
                        h.corpo.queue_draw();
                        h.atualizar_rotulos();
                    }
                });
            }
            glib::ControlFlow::Continue
        });
        let w = Rc::downgrade(&h);
        glib::timeout_add_seconds_local(1, move || {
            let Some(h) = w.upgrade() else { return glib::ControlFlow::Break };
            if h.foco_rodando.get() {
                let r = h.foco_restante.get().saturating_sub(1);
                h.foco_restante.set(r);
                if r == 0 {
                    h.foco_rodando.set(false);
                    h.sessoes.set((h.sessoes.get() + 1).min(4));
                }
                h.corpo.queue_draw();
                h.atualizar_rotulos();
                // focus mode follows the timer (its end brings the session's summary)
                crate::focus_session::changed();
            }
            glib::ControlFlow::Continue
        });
        h
    }

    pub fn ligar(self: &Rc<Self>, shell: &Rc<Shell>) {
        *self.shell.borrow_mut() = Rc::downgrade(shell);
        let w = Rc::downgrade(self);
        shell.midia.ouvir(move |_| {
            if let Some(h) = w.upgrade() {
                if h.painel.aberta.get() {
                    h.render(false);
                }
            }
        });
    }

    pub fn alternar(self: &Rc<Self>) {
        if self.painel.aberta.get() {
            self.close();
        } else {
            self.painel.abrir();
            self.render(true);
            self.refresh_weather();
        }
    }

    fn close(&self) {
        if self.fixada.get() {
            self.set_pinned(false);
        }
        self.editando.set(false);
        self.painel.fechar();
    }

    pub fn aberta(&self) -> bool {
        self.painel.aberta.get()
    }

    /// Opens it pinned (wide screens).
    pub fn pin_open(self: &Rc<Self>) {
        if !self.painel.aberta.get() {
            self.alternar();
        }
        self.set_pinned(true);
    }

    /// The width changed in Settings.
    pub fn width_changed(&self) {
        self.painel.aplicar_largura(self.fixada.get());
    }

    /// Closes the bar if it floats over the windows (open and not pinned).
    pub fn close_if_floating(&self) {
        if self.painel.aberta.get() && !self.fixada.get() {
            self.close();
        }
    }

    fn set_pinned(&self, on: bool) {
        self.fixada.set(on);
        self.painel.place(on);
        crate::conversa::set_icon(&self.alfinete.child().and_downcast::<gtk::Image>().unwrap(), "alfinete", if on { ACCENT } else { MUTED });
        if on { self.alfinete.add_css_class("ativo") } else { self.alfinete.remove_css_class("ativo") }
    }

    /// Reads the forecast again when the bar opens (at most every 30 minutes), only if a city was chosen.
    fn refresh_weather(self: &Rc<Self>) {
        let Some(city) = clima::city() else {
            self.clima.borrow_mut().take();
            return;
        };
        let now = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
        if let Some((c, _, t)) = self.clima.borrow().as_ref() {
            if *c == city && now - t < 30 * 60 {
                return;
            }
        }
        let w = Rc::downgrade(self);
        em_segundo_plano(move || clima::forecast(&city).map(|f| (city, f)), move |r| {
            let Some(h) = w.upgrade() else { return };
            match r {
                Ok((c, f)) => {
                    *h.clima.borrow_mut() = Some((c, f, now));
                    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                        eprintln!("debug hoje: clima lido");
                    }
                    if h.painel.aberta.get() {
                        h.render(false);
                    }
                }
                Err(e) => eprintln!("kalango-shell: clima: {e}"),
            }
        });
    }

    fn atualizar_rotulos(&self) {
        let mut pilha = vec![self.corpo.clone().upcast::<gtk::Widget>()];
        while let Some(w) = pilha.pop() {
            if let Some(l) = w.downcast_ref::<gtk::Label>() {
                match l.widget_name().as_str() {
                    "cpu" => l.set_text(&format!("{:.0}%", self.cpu_uso.get() * 100.0)),
                    "gpu" => l.set_text(&self.gpu.borrow().as_ref().map(|g| format!("{}%", g.0)).unwrap_or("—".into())),
                    "gpu-sub" => l.set_text(&self.gpu.borrow().as_ref().map(|g| format!("{} · {} °C", g.2, g.1)).unwrap_or_else(|| tr("no NVIDIA GPU").into())),
                    "foco" => l.set_text(&format!("{:02}:{:02}", self.foco_restante.get() / 60, self.foco_restante.get() % 60)),
                    "foco-sub" => l.set_text(if self.foco_rodando.get() { tr("left") } else if self.foco_restante.get() == self.foco_total.get() { tr("ready") } else { tr("paused") }),
                    _ => {}
                }
            }
            let mut c = w.first_child();
            while let Some(x) = c {
                c = x.next_sibling();
                pilha.push(x);
            }
        }
    }

    /// A widget, with its × while editing.
    fn widget(self: &Rc<Self>, id: &'static str, w: gtk::Widget) -> gtk::Widget {
        if !self.editando.get() {
            return w;
        }
        let o = gtk::Overlay::new();
        let moldura = caixa(Orientation::Vertical, 0, "hoje-editando");
        moldura.append(&w);
        o.set_child(Some(&moldura));
        let x = gtk::Button::new();
        x.set_child(Some(&ui_icon("fechar", TEXT, 16)));
        x.add_css_class("hoje-remover");
        x.set_widget_name(&format!("hoje:remover {id}"));
        x.set_tooltip_text(Some(tr("Remove from the bar")));
        x.set_halign(Align::End);
        x.set_valign(Align::Start);
        let me = Rc::downgrade(self);
        x.connect_clicked(move |_| {
            if let Some(h) = me.upgrade() {
                let mut oc = hidden_widgets();
                if !oc.iter().any(|x| x == id) {
                    oc.push(id.to_string());
                }
                save_hidden(&oc);
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug hoje: widget {id} removido");
                }
                h.render(false);
            }
        });
        o.add_overlay(&x);
        o.upcast()
    }

    fn render(self: &Rc<Self>, cascata: bool) {
        let Some(shell) = self.shell.borrow().upgrade() else { return };
        limpar(&self.corpo);
        let ocultos = hidden_widgets();
        let mostra = |id: &str| !ocultos.iter().any(|x| x == id);
        let mut blocos: Vec<gtk::Widget> = Vec::new();

        if mostra("resumo") {
            blocos.push(self.widget("resumo", self.resumo(&shell).upcast()));
        }
        // two columns: weather | focus, CPU | GPU (one alone takes the row)
        for par in [["clima", "foco"], ["cpu", "gpu"]] {
            let linha = caixa(Orientation::Horizontal, 16, "");
            linha.set_homogeneous(true);
            for id in par {
                if !mostra(id) {
                    continue;
                }
                let w: gtk::Widget = match id {
                    "clima" => self.clima_widget().upcast(),
                    "foco" => self.foco().upcast(),
                    "cpu" => {
                        let (nuc, ghz) = sistema::nucleos_ghz();
                        let me = Rc::downgrade(self);
                        anel("CPU", "cpu", "cpu", &trf("{n} cores · {ghz} GHz", &[("n", nuc.to_string()), ("ghz", format!("{ghz:.1}"))]), "", move || me.upgrade().map(|h| h.cpu_uso.get()).unwrap_or(0.0), TEAL).upcast()
                    }
                    _ => {
                        let me = Rc::downgrade(self);
                        anel("GPU", "monitor", "gpu", "", "gpu-sub", move || me.upgrade().and_then(|h| h.gpu.borrow().as_ref().map(|g| g.0 as f64 / 100.0)).unwrap_or(0.0), ACCENT).upcast()
                    }
                };
                let id: &'static str = WIDGETS.iter().find(|(w, _)| *w == id).map(|(w, _)| *w).unwrap_or("cpu");
                linha.append(&self.widget(id, w));
            }
            if linha.first_child().is_some() {
                blocos.push(linha.upcast());
            }
            if par[0] == "clima" && mostra("agenda") {
                blocos.push(self.widget("agenda", self.agenda().upcast()));
            }
        }
        if mostra("midia") {
            if let Some(m) = shell.midia.atual() {
                blocos.push(self.widget("midia", cartao_midia(&shell, &m).upcast()));
            }
        }
        let mut animaveis = Vec::new();
        for b in blocos {
            let a = Animavel::new(&b);
            self.corpo.append(&a);
            animaveis.push(a);
        }
        if cascata {
            mv::cascata(&animaveis, 8.0);
        }
        self.render_rodape(&ocultos);
        self.atualizar_rotulos();
        // test aid: where the focus ring, "Edit widgets" and the × buttons are (the bar is at the left edge, 8 px in)
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            let (corpo, rodape, janela) = (self.corpo.clone(), self.rodape.clone(), self.painel.janela.clone());
            glib::timeout_add_local_once(Duration::from_millis(900), move || {
                let dy = f64::from(janela.margin(Edge::Top) + if janela.exclusive_zone() == -1 { 0 } else { BARRA_ALTURA });
                let dx = 8.0;
                let mut pilha: Vec<gtk::Widget> = vec![corpo.upcast(), rodape.upcast()];
                while let Some(w) = pilha.pop() {
                    let nome = w.widget_name();
                    if let Some(item) = nome.strip_prefix("hoje:") {
                        if let Some(b) = w.compute_bounds(&janela) {
                            eprintln!("debug hoje: {} em {} {}", item, (dx + f64::from(b.x() + b.width() / 2.)) as i32, (dy + f64::from(b.y() + b.height() / 2.)) as i32);
                        }
                    }
                    let mut c = w.first_child();
                    while let Some(x) = c {
                        c = x.next_sibling();
                        pilha.push(x);
                    }
                }
            });
        }
    }

    /// "Edit widgets" (× on each one) and "Add" (the hidden ones come back).
    fn render_rodape(self: &Rc<Self>, ocultos: &[String]) {
        limpar(&self.rodape);
        let editar = gtk::Button::new();
        let eb = caixa(Orientation::Horizontal, 6, "");
        eb.set_halign(Align::Center);
        eb.append(&ui_icon(if self.editando.get() { "check" } else { "lapis" }, TEXT, 16));
        eb.append(&rotulo(if self.editando.get() { tr("Done") } else { tr("Edit widgets") }, "hoje-rodape-texto"));
        editar.set_child(Some(&eb));
        editar.add_css_class("hoje-editar");
        editar.set_widget_name("hoje:editar");
        editar.set_hexpand(true);
        let me = Rc::downgrade(self);
        editar.connect_clicked(move |_| {
            if let Some(h) = me.upgrade() {
                h.editando.set(!h.editando.get());
                h.render(false);
            }
        });
        self.rodape.append(&editar);
        let adicionar = gtk::MenuButton::new();
        let ab = caixa(Orientation::Horizontal, 6, "");
        ab.append(&ui_icon("mais", ACCENT, 16));
        ab.append(&rotulo(tr("Add"), "hoje-adicionar-texto"));
        adicionar.set_child(Some(&ab));
        adicionar.set_always_show_arrow(false);
        adicionar.add_css_class("hoje-adicionar");
        let pop = gtk::Popover::new();
        pop.add_css_class("menu");
        let lista = caixa(Orientation::Vertical, 0, "");
        let disponiveis: Vec<&(&str, &str)> = WIDGETS.iter().filter(|(id, _)| ocultos.iter().any(|x| x == id)).collect();
        if disponiveis.is_empty() {
            let l = rotulo(tr("Every widget is already on the bar."), "fraco");
            l.set_margin_top(8);
            l.set_margin_bottom(8);
            l.set_margin_start(10);
            l.set_margin_end(10);
            lista.append(&l);
        }
        for (id, nome) in disponiveis {
            let b = gtk::Button::with_label(tr(nome));
            b.add_css_class("flat");
            let (me, id, p) = (Rc::downgrade(self), id.to_string(), pop.downgrade());
            b.connect_clicked(move |_| {
                if let Some(p) = p.upgrade() {
                    p.popdown();
                }
                let oc: Vec<String> = hidden_widgets().into_iter().filter(|x| *x != id).collect();
                save_hidden(&oc);
                if let Some(h) = me.upgrade() {
                    h.render(false);
                }
            });
            lista.append(&b);
        }
        pop.set_child(Some(&lista));
        adicionar.set_popover(Some(&pop));
        self.rodape.append(&adicionar);
    }

    /// The day summary (the same one as on the desktop, made by `kalango-agente resumo`).
    fn resumo(self: &Rc<Self>, shell: &Rc<Shell>) -> gtk::Box {
        let c = caixa(Orientation::Vertical, 10, "hoje-widget");
        let v = shell.estado.resumo_dia.borrow().clone();
        let cab = caixa(Orientation::Horizontal, 6, "");
        cab.append(&ui_icon("spark", ACCENT, 16));
        let gerado = v.as_ref().is_some_and(|v| v["gerado"] == true);
        cab.append(&secao(if gerado { tr("DAY SUMMARY · MADE BY THE AGENT") } else { tr("DAY SUMMARY") }));
        c.append(&cab);
        let ola = rotulo(&crate::area::greeting(), "hoje-ola");
        ola.set_xalign(0.0);
        c.append(&ola);
        match &v {
            Some(v) => {
                for it in v["itens"].as_array().into_iter().flatten() {
                    let row = caixa(Orientation::Horizontal, 10, "");
                    let (glyph, color) = match it["tipo"].as_str() {
                        Some("feito") => ("check", TEAL),
                        Some("aviso") => ("alerta", GOLD),
                        Some("agenda") => ("relogio", "#7CC3DF"),
                        _ => ("raio", MUTED),
                    };
                    let i = ui_icon(glyph, color, 16);
                    i.set_valign(Align::Start);
                    row.append(&i);
                    let t = rotulo(it["texto"].as_str().unwrap_or(""), "hoje-item");
                    t.set_wrap(true);
                    t.set_xalign(0.0);
                    t.set_hexpand(true);
                    t.set_max_width_chars(1);
                    row.append(&t);
                    c.append(&row);
                }
            }
            None => {
                let t = rotulo(tr("Preparing the summary…"), "fraco");
                t.set_xalign(0.0);
                c.append(&t);
                // not loaded yet (the desktop asks a few seconds after login): ask now
                let (estado, me) = (shell.estado.clone(), Rc::downgrade(self));
                em_segundo_plano(|| {
                    std::process::Command::new(crate::voz::agent_binary()).arg("resumo").output().ok().and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
                }, move |v: Option<Value>| {
                    if let Some(v) = v {
                        *estado.resumo_dia.borrow_mut() = Some(v);
                        if let Some(h) = me.upgrade() {
                            if h.painel.aberta.get() {
                                h.render(false);
                            }
                        }
                    }
                });
            }
        }
        let acoes = caixa(Orientation::Horizontal, 8, "");
        acoes.set_homogeneous(true);
        for (text, class) in [(tr("Review pending"), "hoje-botao-principal"), (tr("Open conversation"), "hoje-botao")] {
            let b = gtk::Button::with_label(text);
            b.add_css_class(class);
            let s = Rc::downgrade(shell);
            b.connect_clicked(move |_| {
                if let Some(s) = s.upgrade() {
                    s.right_bar().abrir(Modo::Agente);
                }
            });
            acoes.append(&b);
        }
        c.append(&acoes);
        c
    }

    /// Weather: off until the person picks a city (nothing goes out before that).
    fn clima_widget(self: &Rc<Self>) -> gtk::Box {
        let c = caixa(Orientation::Vertical, 10, "hoje-widget");
        let noite = glib::DateTime::now_local().map(|d| !(6..18).contains(&d.hour())).unwrap_or(false);
        let cab = caixa(Orientation::Horizontal, 6, "");
        cab.append(&ui_icon(if noite { "lua-pequena" } else { "sol" }, MUTED, 16));
        let atual = self.clima.borrow().clone();
        let titulo = match &atual {
            Some((city, _, _)) => format!("{} · {}", tr("WEATHER"), city.name.to_uppercase()),
            None => tr("WEATHER").to_string(),
        };
        let l = secao(&titulo);
        l.set_halign(Align::Fill);
        l.set_xalign(0.0);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l.set_max_width_chars(1);
        l.set_hexpand(true);
        cab.append(&l);
        c.append(&cab);
        match atual {
            Some((city, f, _)) => {
                let agora = caixa(Orientation::Horizontal, 8, "");
                agora.append(&rotulo(&format!("{:.0}°", f.temp), "hoje-temp"));
                let d = rotulo(clima::describe(f.code), "hoje-sub");
                d.set_valign(Align::End);
                d.set_margin_bottom(8);
                agora.append(&d);
                c.append(&agora);
                let horas = caixa(Orientation::Horizontal, 0, "");
                horas.set_homogeneous(true);
                for (h, t) in &f.hours {
                    let col = caixa(Orientation::Vertical, 2, "");
                    col.append(&rotulo(&format!("{t:.0}°"), "hoje-hora-temp"));
                    col.append(&rotulo(&format!("{h:02}h"), "hoje-hora"));
                    horas.append(&col);
                }
                c.append(&horas);
                let mm = rotulo(&format!("{} {:.0}° · {} {:.0}°", tr("max"), f.max, tr("min"), f.min), "hoje-sub");
                mm.set_xalign(0.0);
                c.append(&mm);
                let gesto = gtk::GestureClick::new();
                gesto.set_button(3);
                c.set_tooltip_text(Some(&format!("{}, {} · {}", city.name, city.region, tr("right-click to change the city"))));
                let me = Rc::downgrade(self);
                let cc = c.clone();
                gesto.connect_pressed(move |_, _, _, _| {
                    if let Some(h) = me.upgrade() {
                        h.escolher_cidade(&cc);
                    }
                });
                c.add_controller(gesto);
            }
            None => {
                let t = rotulo(tr("Pick your city to see the weather. It uses the internet (Open-Meteo) with only the city's location."), "hoje-sub");
                t.set_wrap(true);
                t.set_xalign(0.0);
                t.set_max_width_chars(1);
                t.set_hexpand(true);
                c.append(&t);
                let b = gtk::Button::with_label(tr("Pick city"));
                b.add_css_class("hoje-botao");
                b.set_halign(Align::Start);
                let me = Rc::downgrade(self);
                let cc = c.clone();
                b.connect_clicked(move |_| {
                    if let Some(h) = me.upgrade() {
                        h.escolher_cidade(&cc);
                    }
                });
                c.append(&b);
            }
        }
        c
    }

    /// A small search for the city (geocoding by Open-Meteo); the chosen one is kept in ~/.config/kalango/clima.json.
    fn escolher_cidade(self: &Rc<Self>, ancora: &gtk::Box) {
        let pop = gtk::Popover::new();
        pop.set_parent(ancora);
        let col = caixa(Orientation::Vertical, 6, "hoje-cidade");
        // (as wide as the bar allows: the popover stays inside it, its texts wrap)
        let largura = ancora.root().map(|r| r.width()).filter(|w| *w > 0).unwrap_or(360);
        col.set_size_request((largura - 56).clamp(200, 340), -1);
        let e = gtk::Entry::new();
        e.set_placeholder_text(Some(tr("City, state")));
        col.append(&e);
        let res = caixa(Orientation::Vertical, 0, "");
        col.append(&res);
        if clima::city().is_some() {
            let off = gtk::Button::with_label(tr("Turn weather off"));
            off.add_css_class("flat");
            let (me, p) = (Rc::downgrade(self), pop.downgrade());
            off.connect_clicked(move |_| {
                let _ = clima::set_city(None);
                if let Some(p) = p.upgrade() {
                    p.popdown();
                }
                if let Some(h) = me.upgrade() {
                    h.clima.borrow_mut().take();
                    h.render(false);
                }
            });
            col.append(&off);
        }
        pop.set_child(Some(&col));
        let (me, p, r2) = (Rc::downgrade(self), pop.downgrade(), res.clone());
        e.connect_activate(move |e| {
            let nome = e.text().to_string();
            if nome.trim().is_empty() {
                return;
            }
            limpar(&r2);
            r2.append(&rotulo(tr("Searching…"), "fraco"));
            let lang = kalango_i18n::system_locale().map(|l| l.chars().take(2).collect::<String>()).unwrap_or_else(|| "en".into());
            let (me, p, r3) = (me.clone(), p.clone(), r2.clone());
            em_segundo_plano(move || clima::search(&nome, &lang), move |r| {
                limpar(&r3);
                match r {
                    Ok(v) if !v.is_empty() => {
                        for city in v {
                            let b = gtk::Button::with_label(&format!("{} · {}", city.name, city.region));
                            b.add_css_class("flat");
                            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                                l.set_xalign(0.0);
                            }
                            let (me, p) = (me.clone(), p.clone());
                            b.connect_clicked(move |_| {
                                let _ = clima::set_city(Some(&city));
                                if let Some(p) = p.upgrade() {
                                    p.popdown();
                                }
                                if let Some(h) = me.upgrade() {
                                    h.clima.borrow_mut().take();
                                    h.refresh_weather();
                                }
                            });
                            r3.append(&b);
                        }
                    }
                    Ok(_) => r3.append(&quebra(rotulo(tr("No city with that name."), "fraco"))),
                    Err(e) => {
                        eprintln!("kalango-shell: clima: {e}");
                        let texto = if ["dns", "connect", "resolve", "network", "timed out"].iter().any(|k| e.to_lowercase().contains(k)) {
                            tr("No internet connection: the city could not be looked up.")
                        } else {
                            tr("The weather service did not answer: try again in a moment.")
                        };
                        r3.append(&quebra(rotulo(texto, "fraco")));
                    }
                }
            });
        });
        pop.connect_closed(|p| p.unparent());
        pop.popup();
        e.grab_focus();
    }

    /// Today's events from the local calendars (read only).
    fn agenda(self: &Rc<Self>) -> gtk::Box {
        let c = caixa(Orientation::Vertical, 10, "hoje-widget");
        let cab = caixa(Orientation::Horizontal, 6, "");
        cab.append(&ui_icon("relogio", MUTED, 16));
        let hoje = glib::DateTime::now_local().ok();
        let dia = hoje.as_ref().and_then(|d| d.format("%A %-d").ok()).map(|s| s.to_uppercase()).unwrap_or_default();
        cab.append(&secao(&format!("{} · {dia}", tr("TODAY"))));
        c.append(&cab);
        let eventos = agenda::today();
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug hoje: agenda com {} eventos", eventos.len());
        }
        if eventos.is_empty() {
            let t = rotulo(tr("Nothing on the calendar today."), "hoje-sub");
            t.set_xalign(0.0);
            c.append(&t);
        }
        for (i, e) in eventos.iter().take(5).enumerate() {
            let row = caixa(Orientation::Horizontal, 10, "");
            let cor = caixa(Orientation::Vertical, 0, "hoje-evento-cor");
            cor.add_css_class(["acento", "verde", "ouro"][i % 3]);
            row.append(&cor);
            let col = caixa(Orientation::Vertical, 1, "");
            col.set_hexpand(true);
            let t = rotulo(&e.title, "hoje-evento");
            t.set_xalign(0.0);
            t.set_ellipsize(gtk::pango::EllipsizeMode::End);
            t.set_max_width_chars(1);
            t.set_hexpand(true);
            col.append(&t);
            let h = rotulo(&e.start.map(|m| format!("{:02}:{:02}", m / 60, m % 60)).unwrap_or_else(|| tr("all day").to_string()), "hoje-sub");
            h.set_xalign(0.0);
            col.append(&h);
            row.append(&col);
            c.append(&row);
        }
        c
    }

    /// The focus timer the whole system shares (this bar and the desktop widget): seconds left, total, running,
    /// sessions finished today.
    pub(crate) fn foco_estado(&self) -> (u32, u32, bool, u32) {
        (self.foco_restante.get(), self.foco_total.get(), self.foco_rodando.get(), self.sessoes.get())
    }

    /// Starts or pauses the focus timer (after the end, it starts over).
    pub(crate) fn foco_alternar(&self) {
        if self.foco_restante.get() == 0 {
            self.foco_restante.set(self.foco_total.get());
        }
        self.foco_rodando.set(!self.foco_rodando.get());
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug hoje: foco rodando={}", self.foco_rodando.get());
        }
        self.corpo.queue_draw();
        self.atualizar_rotulos();
        crate::focus_session::changed();
    }

    /// Back to the start (stopped).
    pub(crate) fn foco_zerar(&self) {
        self.foco_rodando.set(false);
        self.foco_restante.set(self.foco_total.get());
        self.corpo.queue_draw();
        self.atualizar_rotulos();
        crate::focus_session::changed();
    }

    fn foco(self: &Rc<Self>) -> gtk::Box {
        let w = Rc::downgrade(self);
        let c = anel(tr("FOCUS"), "relogio", "foco", "", "foco-sub", move || {
            w.upgrade().map(|h| h.foco_restante.get() as f64 / h.foco_total.get() as f64).unwrap_or(1.0)
        }, ACCENT);
        // design: the ring is the control. Click starts or pauses; a double click resets
        c.set_tooltip_text(Some(tr("Click to start or pause; double click to reset")));
        c.set_widget_name("hoje:foco");
        let g = gtk::GestureClick::new();
        let w = Rc::downgrade(self);
        g.connect_pressed(move |_, n, _, _| {
            let Some(h) = w.upgrade() else { return };
            if n >= 2 {
                h.foco_zerar();
            } else {
                h.foco_alternar();
            }
        });
        c.add_controller(g);
        let sub = rotulo(&trf("session {k} of 4", &[("k", (self.sessoes.get() + 1).min(4).to_string())]), "hoje-sub");
        sub.set_xalign(0.0);
        c.append(&sub);
        c
    }
}

/// Indicador em anel (CPU, GPU, foco) desenhado com cairo; `valor` é lido a cada quadro desenhado.
fn anel(titulo: &str, glifo: &str, id_valor: &str, sub: &str, id_sub: &str, valor: impl Fn() -> f64 + 'static, cor: &str) -> gtk::Box {
    let c = caixa(Orientation::Vertical, 10, "hoje-widget");
    let cab = caixa(Orientation::Horizontal, 6, "");
    cab.append(&ui_icon(glifo, MUTED, 16));
    cab.append(&secao(titulo));
    c.append(&cab);
    let area = gtk::DrawingArea::new();
    area.set_content_width(96);
    area.set_content_height(96);
    area.set_halign(Align::Start);
    let cor = cor.to_string();
    area.set_draw_func(move |_, cr, w, h| {
        let rgba = gdk::RGBA::parse(crate::tema::cor(&cor)).unwrap_or(gdk::RGBA::WHITE);
        let (cx, cy, r) = (w as f64 / 2.0, h as f64 / 2.0, (w.min(h) as f64) / 2.0 - 6.0);
        cr.set_line_width(7.0);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        let (fr, fg, fb) = crate::tema::rgb("#FFFFFF");
        cr.set_source_rgba(fr, fg, fb, 0.08);
        cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
        let _ = cr.stroke();
        let v = valor().clamp(0.0, 1.0);
        if v > 0.0 {
            cr.set_source_rgba(rgba.red() as f64, rgba.green() as f64, rgba.blue() as f64, 1.0);
            let ini = -std::f64::consts::FRAC_PI_2;
            cr.arc(cx, cy, r, ini, ini + v * std::f64::consts::TAU);
            let _ = cr.stroke();
        }
    });
    let sobre = gtk::Overlay::new();
    sobre.set_child(Some(&area));
    sobre.set_halign(Align::Start);
    let meio = caixa(Orientation::Vertical, 0, "");
    meio.set_halign(Align::Center);
    meio.set_valign(Align::Center);
    let v = rotulo("—", "anel-valor");
    v.set_widget_name(id_valor);
    meio.append(&v);
    if !id_sub.is_empty() && id_valor == "foco" {
        let s = rotulo("", "hoje-sub");
        s.set_widget_name(id_sub);
        meio.append(&s);
    }
    sobre.add_overlay(&meio);
    c.append(&sobre);
    let sl = rotulo(sub, "hoje-sub");
    sl.set_halign(Align::Start);
    if !id_sub.is_empty() && id_valor != "foco" {
        sl.set_widget_name(id_sub);
    }
    if !sub.is_empty() || (!id_sub.is_empty() && id_valor != "foco") {
        c.append(&sl);
    }
    // redesenha o anel quando o valor muda (o laço de 1–1,5 s chama queue_draw no corpo)
    let a2 = area.clone();
    glib::timeout_add_local(Duration::from_millis(500), move || {
        if a2.root().is_none() {
            return glib::ControlFlow::Break;
        }
        a2.queue_draw();
        glib::ControlFlow::Continue
    });
    let _ = BARRA_ALTURA;
    c
}

/// A label that wraps inside its box instead of widening it.
fn quebra(l: gtk::Label) -> gtk::Label {
    l.set_wrap(true);
    l.set_max_width_chars(1);
    l.set_xalign(0.0);
    l.set_hexpand(true);
    l
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn e(tool: &str, status: &str) -> Value {
        json!({"tool": tool, "status": status, "summary": format!("{tool} {status}"), "time": 0})
    }

    #[test]
    fn the_same_tool_in_a_row_is_one_line() {
        let l = vec![e("web.buscar", "ok"), e("web.buscar", "ok"), e("web.buscar", "ok"), e("web.ler", "ok"), e("web.buscar", "ok")];
        let g = group_timeline(&l);
        assert_eq!(g, vec![
            TimelineGroup { tool: "web.buscar".into(), first: 0, len: 3 },
            TimelineGroup { tool: "web.ler".into(), first: 3, len: 1 },
            TimelineGroup { tool: "web.buscar".into(), first: 4, len: 1 },
        ]);
        assert!(group_timeline(&[]).is_empty());
    }

    #[test]
    fn a_failure_or_a_denial_starts_its_own_line() {
        let l = vec![e("arquivos.mover", "ok"), e("arquivos.mover", "erro"), e("arquivos.mover", "ok"), e("arquivos.mover", "ok"), e("arquivos.mover", "negado")];
        let lens: Vec<usize> = group_timeline(&l).iter().map(|g| g.len).collect();
        assert_eq!(lens, [1, 1, 2, 1]);
        // entries without a tool never join
        assert_eq!(group_timeline(&[json!({}), json!({})]).len(), 2);
    }

    #[test]
    fn a_line_says_what_the_tool_did() {
        assert_eq!(timeline_words("arquivos.mover", 1, "Moveu nota.txt", true), ("Moveu nota.txt".to_string(), None));
        assert_eq!(timeline_words("web.buscar", 9, "x", true), (tr("Searched the web").to_string(), Some(trf("{n} searches", &[("n", "9".into())]))));
        assert_eq!(timeline_words("arquivos.mover", 3, "x", true).0, trf("Moved {n} items", &[("n", "3".into())]));
        assert_eq!(timeline_words("outra.coisa", 4, "x", true), ("outra.coisa".to_string(), Some("4".to_string())));
        // (what failed is not told as done)
        assert_eq!(timeline_words("arquivos.mover", 2, "x", false), ("arquivos.mover".to_string(), Some("2".to_string())));
    }
}
