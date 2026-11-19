//! Overview (design "05 · Telas do sistema e apps", frame "Visão geral (janelas e espaços)"): the workspaces of the
//! output as small pictures (with their names, renamed with a double click, and "+" for a new one), the universal
//! search, the windows of the workspace as pictures (click to go, middle click or × to close, drag onto a workspace
//! to move it there) and, on the right, the agents at work and the requests waiting for the person.
//!
//! Motion: the windows shrink from where they are into their pictures (gentle spring), the workspaces slide in and
//! the agents' panel comes in from the right; closing takes the windows back to their places.

use crate::alternador::{carregar_miniatura, thumbs_dir};
use gtk::gdk_pixbuf::{InterpType, Pixbuf};
use crate::apps;
use crate::conversa::ui_icon;
use crate::estado::Janela;
use crate::servicos::{self, niri_acao};
use crate::superficies::{caixa, camada, entrada_sem_furos, furos, rotulo, veu, Furo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use kalango_movimento::{self as mv, Animavel, Canal};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

/// The workspaces' colour dots, in order (accent, teal, gold, coral).
pub(crate) const DOTS: [&str; 4] = ["#A5A1E2", "#8ED2C3", "#E5C98C", "#EFA2A6"];
const GAP: f64 = 40.0;
/// the label under a window picture: 8 px of space and 26 of chip
const LABEL: f64 = 34.0;
const WS_W: f64 = 200.0;
const WS_H: f64 = 112.0;
/// a request's steps are bounded by the engine (motor.rs MAX_STEPS)
const MAX_STEPS: f64 = 8.0;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// A window picture on screen: the window, its spring, where the picture is and where the window really is.
type Cartao = (u64, Animavel, Lugar, Option<Lugar>);

/// Where a window picture goes in the windows' area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Lugar {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) w: f64,
    pub(crate) h: f64,
}

/// Lays out pictures of these aspect ratios (width / height, with each window's real width as the largest size)
/// in rows or in columns, whichever shows them biggest, centred in `aw` × `ah`.
fn arranjar(janelas: &[(f64, f64)], aw: f64, ah: f64) -> Vec<Lugar> {
    let n = janelas.len();
    if n == 0 || aw <= 0.0 || ah <= 0.0 {
        return vec![];
    }
    let mut melhor: Option<(f64, Vec<Lugar>)> = None;
    for k in 1..=n {
        // k groups in order; the later groups take the extra ones (the first window gets the most room)
        let (base, resto) = (n / k, n % k);
        let mut grupos = Vec::new();
        let mut i = 0;
        for g in 0..k {
            let c = base + usize::from(g >= k - resto);
            grupos.push(i..i + c);
            i += c;
        }
        for linhas in [true, false] {
            let mut lugares = vec![Lugar { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }; n];
            if linhas {
                // each row fills the width; then everything is scaled to fit the height
                let mut y = 0.0;
                for g in &grupos {
                    let soma: f64 = janelas[g.clone()].iter().map(|j| j.0).sum();
                    let h = (aw - GAP * (g.len() as f64 - 1.0)) / soma;
                    let mut x = 0.0;
                    for i in g.clone() {
                        lugares[i] = Lugar { x, y, w: h * janelas[i].0, h };
                        x += h * janelas[i].0 + GAP;
                    }
                    y += h + LABEL + GAP;
                }
            } else {
                // each column fills the height; then everything is scaled to fit the width
                let mut x = 0.0;
                for g in &grupos {
                    let soma: f64 = janelas[g.clone()].iter().map(|j| 1.0 / j.0).sum();
                    let w = (ah - LABEL * g.len() as f64 - GAP * (g.len() as f64 - 1.0)).max(1.0) / soma;
                    let mut y = 0.0;
                    for i in g.clone() {
                        lugares[i] = Lugar { x, y, w, h: w / janelas[i].0 };
                        y += w / janelas[i].0 + LABEL + GAP;
                    }
                    x += w + GAP;
                }
            }
            let largura = lugares.iter().map(|l| l.x + l.w).fold(0.0, f64::max);
            let altura = lugares.iter().map(|l| l.y + l.h + LABEL).fold(0.0, f64::max);
            // fit, and never bigger than the window itself
            let mut s = (aw / largura).min(ah / altura).min(1.0);
            for (l, j) in lugares.iter().zip(janelas) {
                s = s.min(j.1 / l.w);
            }
            for l in lugares.iter_mut() {
                *l = Lugar { x: l.x * s, y: l.y * s, w: l.w * s, h: l.h * s };
            }
            // centre: rows each on its own, columns each on their own, then the whole block
            if linhas {
                for g in &grupos {
                    let fim = lugares[g.end - 1].x + lugares[g.end - 1].w;
                    let dx = (aw - fim) / 2.0;
                    for i in g.clone() {
                        lugares[i].x += dx;
                    }
                }
                let alt = lugares.iter().map(|l| l.y + l.h + LABEL).fold(0.0, f64::max);
                for l in lugares.iter_mut() {
                    l.y += (ah - alt) / 2.0;
                }
            } else {
                for g in &grupos {
                    let fim = lugares[g.end - 1].y + lugares[g.end - 1].h + LABEL;
                    let dy = (ah - fim) / 2.0;
                    for i in g.clone() {
                        lugares[i].y += dy;
                    }
                }
                let larg = lugares.iter().map(|l| l.x + l.w).fold(0.0, f64::max);
                for l in lugares.iter_mut() {
                    l.x += (aw - larg) / 2.0;
                }
            }
            let area: f64 = lugares.iter().map(|l| l.w * l.h).sum();
            if melhor.as_ref().is_none_or(|(a, _)| area > *a + 1.0) {
                melhor = Some((area, lugares));
            }
        }
    }
    melhor.map(|(_, l)| l).unwrap_or_default()
}

/// A picture of exactly this size (a Picture's natural size is its image's: scaled first).
pub(crate) fn figura(pb: &Pixbuf, w: i32, h: i32) -> gtk::Picture {
    let (w, h) = (w.max(1), h.max(1));
    let pb = pb.scale_simple(w, h, InterpType::Bilinear).unwrap_or_else(|| pb.clone());
    #[allow(deprecated)]
    let p = gtk::Picture::for_paintable(&gdk::Texture::for_pixbuf(&pb));
    p.set_content_fit(gtk::ContentFit::Fill);
    p.set_size_request(w, h);
    p
}

/// The output's wallpaper cut to the workspaces' proportion, small (the 4K file is decoded once).
pub(crate) fn papel_pequeno(ow: i32, oh: i32) -> Option<Pixbuf> {
    let p = crate::papel::para(ow, oh)?;
    let pb = Pixbuf::from_file_at_scale(p, (WS_W * 2.0) as i32, -1, true).ok()?;
    let alvo = (WS_H / WS_W * f64::from(pb.width())) as i32;
    if pb.height() > alvo {
        Some(pb.new_subpixbuf(0, (pb.height() - alvo) / 2, pb.width(), alvo))
    } else {
        Some(pb)
    }
}

/// Where the windows of one workspace are on the output. Floating windows: niri says. Tiled ones: niri gives
/// their column and size only, so the columns are laid as niri does (no gaps: niri.kdl's "gaps 0", below the bar), with the column
/// focused last kept in view.
pub(crate) fn lugares_reais(janelas: &[&Janela], ow: f64) -> HashMap<u64, Lugar> {
    const GAPS: f64 = 0.0;
    let topo = f64::from(crate::superficies::BARRA_ALTURA) + GAPS;
    let mut m = HashMap::new();
    let mut colunas: std::collections::BTreeMap<usize, Vec<&Janela>> = Default::default();
    for j in janelas {
        match (j.pos, j.coluna) {
            (Some((x, y)), _) => {
                m.insert(j.id, Lugar { x, y, w: f64::from(j.tamanho.0.max(1)), h: f64::from(j.tamanho.1.max(1)) });
            }
            (None, Some((c, _))) => colunas.entry(c).or_default().push(j),
            _ => {}
        }
    }
    let mut x = GAPS;
    let mut tiled = Vec::new();
    for col in colunas.values_mut() {
        col.sort_by_key(|j| j.coluna.map(|c| c.1));
        let largura = col.iter().map(|j| j.tile.0).fold(0.0, f64::max);
        let mut y = topo;
        for j in col.iter() {
            tiled.push((j.id, j.foco_em, Lugar { x, y, w: j.tile.0.max(1.0), h: j.tile.1.max(1.0) }));
            y += j.tile.1 + GAPS;
        }
        x += largura + GAPS;
    }
    // niri scrolls the view to keep the focused column on screen
    let deslocar = tiled.iter().max_by_key(|t| t.1).map(|t| (t.2.x + t.2.w + GAPS - ow).max(0.0)).unwrap_or(0.0);
    for (id, _, l) in tiled {
        m.insert(id, Lugar { x: l.x - deslocar, ..l });
    }
    m
}

/// "18k tokens", "640 tokens".
fn tokens_text(n: u64) -> String {
    if n >= 1000 {
        trf("{n}k tokens", &[("n", ((n + 500) / 1000).to_string())])
    } else {
        trf("{n} tokens", &[("n", n.to_string())])
    }
}

fn minutos(s: u64) -> String {
    if s < 60 {
        tr("less than 1 min").to_string()
    } else {
        trf("{n} min", &[("n", (s / 60).to_string())])
    }
}

pub struct Visao {
    janela: gtk::ApplicationWindow,
    fundo: Animavel,
    buracos: Rc<RefCell<Vec<Furo>>>,
    espacos: gtk::Box,
    espacos_anim: Animavel,
    busca: gtk::Entry,
    area: gtk::Fixed,
    vazio: gtk::Label,
    /// the window pictures now
    cartoes: RefCell<Vec<Cartao>>,
    painel: Animavel,
    agentes: gtk::Box,
    pendentes: RefCell<Vec<Value>>,
    pub aberta: Cell<bool>,
    shell: RefCell<Weak<Shell>>,
    /// the workspace whose windows are shown
    espaco: Cell<u64>,
    /// the size of the output the overview is on (known before the window has its own)
    tela: Cell<(f64, f64)>,
    /// window pictures from niri, by window
    texturas: RefCell<HashMap<u64, Pixbuf>>,
    /// the wallpaper at the workspaces' size, for the output's size (decoded once per size)
    papel: RefCell<Option<((i32, i32), Pixbuf)>>,
    /// what was drawn (to redraw only when windows or workspaces change)
    assinatura: RefCell<String>,
    assinatura_agentes: RefCell<String>,
    /// the workspace tiles (and "+"), for dropping a window on them
    alvos: RefCell<Vec<(gtk::Widget, u64)>>,
    /// a window picture is being dragged (nothing is redrawn under it)
    arrastando: Cell<bool>,
    /// a workspace's name is being edited (Esc cancels that, not the overview)
    renomeando: RefCell<Option<Box<dyn Fn()>>>,
}

impl Visao {
    pub fn new(app: &gtk::Application) -> Rc<Visao> {
        let janela = camada(app, "kalango-visao-geral", Layer::Overlay, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
        janela.set_exclusive_zone(-1);
        let raiz = gtk::Overlay::new();
        let buracos: Rc<RefCell<Vec<Furo>>> = Rc::default();
        let fundo = Animavel::new(&veu(0.72, buracos.clone()));
        raiz.set_child(Some(&fundo));

        // workspaces (y = 52 in the design: 32 of bar + 20)
        let espacos = caixa(Orientation::Horizontal, 14, "");
        let espacos_anim = Animavel::new(&espacos);
        espacos_anim.set_halign(Align::Center);
        espacos_anim.set_valign(Align::Start);
        espacos_anim.set_margin_top(52);
        raiz.add_overlay(&espacos_anim);

        // the universal search (560 × 44 at y = 190)
        let busca_caixa = caixa(Orientation::Horizontal, 8, "visao-busca");
        busca_caixa.set_size_request(560, 44);
        busca_caixa.set_halign(Align::Center);
        busca_caixa.set_valign(Align::Start);
        busca_caixa.set_margin_top(190);
        let sp = ui_icon("spark", "#A5A1E2", 16);
        busca_caixa.append(&sp);
        let busca = gtk::Entry::new();
        busca.set_hexpand(true);
        busca.add_css_class("visao-busca-campo");
        busca.set_placeholder_text(Some(tr("Search apps, files, settings or ask the agent…")));
        busca_caixa.append(&busca);
        let atalho = rotulo("Ctrl K", "visao-atalho");
        atalho.set_valign(Align::Center);
        busca_caixa.append(&atalho);
        raiz.add_overlay(&busca_caixa);

        // the windows of the workspace: from (120, 262) on (it must not cover the workspaces or the search)
        let area = gtk::Fixed::new();
        area.set_halign(Align::Start);
        area.set_valign(Align::Start);
        area.set_margin_start(120);
        area.set_margin_top(262);
        raiz.add_overlay(&area);
        let vazio = rotulo(tr("No windows on this workspace"), "visao-vazio");
        vazio.set_halign(Align::Center);
        vazio.set_valign(Align::Center);
        vazio.set_can_target(false);
        vazio.set_visible(false);
        raiz.add_overlay(&vazio);

        // agents at work (400 wide at x = 1400, y = 270 on 1920)
        let agentes = caixa(Orientation::Vertical, 12, "visao-agentes");
        agentes.set_size_request(400, -1);
        let painel = Animavel::new(&agentes);
        painel.set_halign(Align::End);
        painel.set_valign(Align::Start);
        painel.set_margin_top(270);
        painel.set_margin_end(120);
        raiz.add_overlay(&painel);
        janela.set_child(Some(&raiz));

        let v = Rc::new(Visao {
            janela, fundo, buracos, espacos, espacos_anim, busca, area, vazio, cartoes: RefCell::default(), painel, agentes,
            pendentes: RefCell::default(), aberta: Cell::new(false), shell: RefCell::default(), espaco: Cell::new(0), tela: Cell::new((1920.0, 1080.0)),
            texturas: RefCell::default(), papel: RefCell::default(), assinatura: RefCell::default(), assinatura_agentes: RefCell::default(),
            alvos: RefCell::default(), arrastando: Cell::new(false), renomeando: RefCell::default(),
        });

        // typing goes on in the command bar (the same search everywhere)
        let w = Rc::downgrade(&v);
        v.busca.connect_changed(move |e| {
            let Some(v) = w.upgrade() else { return };
            let t = e.text().to_string();
            if t.trim().is_empty() || !v.aberta.get() {
                return;
            }
            v.fechar(None);
            let shell = v.shell.borrow().upgrade();
            if let Some(s) = shell {
                s.comando().abrir(&t);
            }
        });
        let teclas = gtk::EventControllerKey::new();
        teclas.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(&v);
        teclas.connect_key_pressed(move |_, k, _, m| {
            let Some(v) = w.upgrade() else { return glib::Propagation::Proceed };
            match k {
                gdk::Key::Escape => {
                    let cancelar = v.renomeando.borrow_mut().take();
                    match cancelar {
                        Some(c) => c(),
                        None => v.fechar(None),
                    }
                }
                gdk::Key::k | gdk::Key::K if m.contains(gdk::ModifierType::CONTROL_MASK) => {
                    v.fechar(None);
                    if let Some(s) = v.shell.borrow().upgrade() {
                        s.comando().abrir("");
                    }
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        v.janela.add_controller(teclas);
        // a click on the empty background closes
        let clique = gtk::GestureClick::new();
        let w = Rc::downgrade(&v);
        clique.connect_released(move |_, _, x, y| {
            if let Some(v) = w.upgrade() {
                if debug() {
                    eprintln!("debug visão geral: clique no fundo em {x:.0} {y:.0}");
                }
                v.fechar(None);
            }
        });
        v.fundo.add_controller(clique);
        // the windows' area between the pictures is background too
        let clique = gtk::GestureClick::new();
        let w = Rc::downgrade(&v);
        clique.connect_released(move |g, _, x, y| {
            let Some(v) = w.upgrade() else { return };
            if v.area.pick(x, y, gtk::PickFlags::DEFAULT).is_some_and(|p| p == v.area.clone().upcast::<gtk::Widget>()) {
                g.set_state(gtk::EventSequenceState::Claimed);
                v.fechar(None);
            }
        });
        v.area.add_controller(clique);
        // the agents' panel follows the work, once a second
        let w = Rc::downgrade(&v);
        glib::timeout_add_seconds_local(1, move || {
            let Some(v) = w.upgrade() else { return glib::ControlFlow::Break };
            if v.aberta.get() {
                v.render_agentes();
            }
            glib::ControlFlow::Continue
        });
        v
    }

    pub fn ligar(self: &Rc<Self>, shell: &Rc<Shell>) {
        *self.shell.borrow_mut() = Rc::downgrade(shell);
        let w = Rc::downgrade(self);
        shell.estado.ouvir(move || {
            if let Some(v) = w.upgrade().filter(|v| v.aberta.get()) {
                v.render(false);
            }
        });
    }

    pub fn alternar(self: &Rc<Self>) {
        if self.aberta.get() {
            self.fechar(None);
        } else {
            self.abrir();
        }
    }

    fn janelas_do(&self, espaco: u64) -> Vec<Janela> {
        let Some(s) = self.shell.borrow().upgrade() else { return vec![] };
        let mut v: Vec<Janela> = s.estado.janelas.borrow().values().filter(|j| j.espaco == Some(espaco)).cloned().collect();
        // as on the workspace: columns left to right, then the floating ones
        v.sort_by_key(|j| (j.coluna.unwrap_or((usize::MAX, usize::MAX)), j.id));
        v
    }

    pub fn abrir(self: &Rc<Self>) {
        let Some(s) = self.shell.borrow().upgrade() else { return };
        let focado = {
            let es = s.estado.espacos.borrow();
            es.iter().find(|e| e.focado).or_else(|| es.first()).map(|e| e.id)
        };
        let Some(espaco) = focado else { return };
        self.espaco.set(espaco);
        self.aberta.set(true);
        self.busca.set_text("");
        self.assinatura.borrow_mut().clear();
        self.assinatura_agentes.borrow_mut().clear();
        // pictures of every window on this output (also for the workspaces' small pictures)
        let saida = s.estado.espacos.borrow().iter().find(|e| e.id == espaco).map(|e| e.saida.clone()).unwrap_or_default();
        let ids: Vec<u64> = {
            let es = s.estado.espacos.borrow();
            s.estado.janelas.borrow().values()
                .filter(|j| j.espaco.is_some_and(|e| es.iter().any(|x| x.id == e && x.saida == saida)))
                .map(|j| j.id).take(24).collect()
        };
        for id in ids {
            self.pedir_miniatura(id);
        }
        // on the output of the focused workspace, with its size from the start (the layout and the opening motion
        // need it before the window has one)
        let monitor = gdk::Display::default().and_then(|d| {
            let ms = d.monitors();
            (0..ms.n_items()).filter_map(|i| ms.item(i).and_downcast::<gdk::Monitor>()).find(|m| m.connector().is_some_and(|c| c == saida))
        });
        if let Some(m) = &monitor {
            self.janela.set_monitor(Some(m));
            let g = m.geometry();
            self.tela.set((f64::from(g.width()), f64::from(g.height())));
        }
        self.janela.present();
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(40), move || {
            if let Some(v) = w.upgrade() {
                crate::superficies::take_keyboard(&v.janela);
                v.recortar();
                v.busca.grab_focus();
            }
        });
        // the panel first: the windows' area depends on it
        self.render_agentes();
        self.render(true);
        self.atualizar_pendentes();
        self.fundo.definir(Canal::Opacidade, 0.0);
        self.fundo.ir(Canal::Opacidade, 1.0, mv::SUAVE);
        self.espacos_anim.entrar((0.0, -24.0), 1.0);
        if debug() {
            eprintln!("debug visão geral: aberta no espaço {espaco}");
        }
    }

    /// The bar and the dock stay out of the scrim and keep their clicks (once the window has its size).
    fn recortar(self: &Rc<Self>) {
        let Some(s) = self.shell.borrow().upgrade() else { return };
        let (w, h) = (self.janela.width(), self.janela.height());
        if w == 0 {
            let v = Rc::downgrade(self);
            glib::timeout_add_local_once(Duration::from_millis(30), move || {
                if let Some(v) = v.upgrade().filter(|v| v.aberta.get()) {
                    v.recortar();
                }
            });
            return;
        }
        if debug() {
            eprintln!("debug visão geral: véu {w}x{h} sem {:?}", furos(&s, w, h).iter().map(|f| (f.x as i32, f.y as i32, f.w as i32, f.h as i32)).collect::<Vec<_>>());
        }
        // the real size (if the monitor's was not known): lay out again
        if (f64::from(w), f64::from(h)) != self.tela.get() {
            self.tela.set((f64::from(w), f64::from(h)));
            self.render(false);
        }
        *self.buracos.borrow_mut() = furos(&s, w, h);
        self.fundo.queue_draw();
        if let Some(c) = self.fundo.first_child() {
            c.queue_draw();
        }
        entrada_sem_furos(&self.janela, &self.buracos.borrow());
    }

    /// Closes; with `ir_para`, that window is focused once the pictures are back in place.
    pub fn fechar(&self, ir_para: Option<u64>) {
        if !self.aberta.replace(false) {
            return;
        }
        if let Some(id) = ir_para {
            let _ = niri_acao(json!({"FocusWindow": {"id": id}}));
        }
        self.janela.set_keyboard_mode(KeyboardMode::None);
        self.fundo.ir(Canal::Opacidade, 0.0, mv::RAPIDA);
        self.painel.ir(Canal::Opacidade, 0.0, mv::RAPIDA);
        self.espacos_anim.ir(Canal::Opacidade, 0.0, mv::RAPIDA);
        // the pictures go back to where the windows are
        for (_, a, l, real) in self.cartoes.borrow().iter() {
            match real {
                Some(r) => {
                    a.ir(Canal::X, (r.x + r.w / 2.0) - (l.x + l.w / 2.0), mv::SUAVE).ir(Canal::Y, (r.y + r.h / 2.0) - (l.y + l.h / 2.0), mv::SUAVE)
                        .ir(Canal::Escala, r.w / l.w, mv::SUAVE).ir(Canal::Opacidade, 0.0, mv::SUAVE);
                }
                None => {
                    a.ir(Canal::Opacidade, 0.0, mv::RAPIDA);
                }
            }
        }
        let j = self.janela.downgrade();
        self.fundo.ao_parar(move || {
            if let Some(j) = j.upgrade() {
                j.set_visible(false);
            }
        });
        if debug() {
            eprintln!("debug visão geral: fechada");
        }
    }

    /// Asks niri for a picture of the window; it is used as soon as it is ready.
    fn pedir_miniatura(self: &Rc<Self>, id: u64) {
        let path = thumbs_dir().join(format!("visao-{id}.png"));
        let _ = std::fs::remove_file(&path);
        if niri_acao(json!({"ThumbnailWindow": {"id": id, "path": path.display().to_string(), "max_size": 720}})).is_err() {
            return;
        }
        let w = Rc::downgrade(self);
        let t0 = Instant::now();
        glib::timeout_add_local(Duration::from_millis(30), move || {
            let Some(v) = w.upgrade() else { return glib::ControlFlow::Break };
            if path.is_file() {
                let Some(pb) = carregar_miniatura(&path) else { return glib::ControlFlow::Continue };
                let _ = std::fs::remove_file(&path);
                v.texturas.borrow_mut().insert(id, pb);
                if v.aberta.get() {
                    v.render(false);
                }
                return glib::ControlFlow::Break;
            }
            if t0.elapsed() > Duration::from_secs(2) {
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    /// Workspaces and windows, redrawn only when something about them changed.
    fn render(self: &Rc<Self>, abrindo: bool) {
        let Some(s) = self.shell.borrow().upgrade() else { return };
        let espaco = self.espaco.get();
        let saida = s.estado.espacos.borrow().iter().find(|e| e.id == espaco).map(|e| e.saida.clone()).unwrap_or_default();
        let mut espacos: Vec<_> = s.estado.espacos.borrow().iter().filter(|e| e.saida == saida).cloned().collect();
        espacos.sort_by_key(|e| e.idx);
        let todas: Vec<Janela> = s.estado.janelas.borrow().values().cloned().collect();
        let assinatura = format!("{espaco}|{:?}|{:?}|{:?}|{}x{}",
            espacos.iter().map(|e| (e.id, &e.nome)).collect::<Vec<_>>(),
            todas.iter().map(|j| (j.id, j.espaco, j.focada, &j.app_id)).collect::<Vec<_>>(),
            self.texturas.borrow().keys().collect::<std::collections::BTreeSet<_>>(),
            self.tela.get().0, self.tela.get().1);
        if !abrindo && (*self.assinatura.borrow() == assinatura || self.arrastando.get() || self.renomeando.borrow().is_some()) {
            return;
        }
        *self.assinatura.borrow_mut() = assinatura;
        self.render_espacos(&espacos, &todas);
        self.render_janelas(&s, abrindo);
    }

    fn render_espacos(self: &Rc<Self>, espacos: &[crate::estado::Espaco], todas: &[Janela]) {
        while let Some(c) = self.espacos.first_child() {
            self.espacos.remove(&c);
        }
        self.alvos.borrow_mut().clear();
        let (ow, oh) = self.tela.get();
        let k = WS_W / ow;
        let tela = (ow as i32, oh as i32);
        if self.papel.borrow().as_ref().is_none_or(|(t, _)| *t != tela) {
            *self.papel.borrow_mut() = papel_pequeno(tela.0, tela.1).map(|p| (tela, p));
        }
        let papel = self.papel.borrow().as_ref().map(|(_, p)| p.clone());
        if debug() {
            eprintln!("debug visão geral: espaços {:?}", espacos.iter().map(|e| (e.id, e.idx, todas.iter().filter(|j| j.espaco == Some(e.id)).map(|j| (j.id, j.pos)).collect::<Vec<_>>())).collect::<Vec<_>>());
        }
        for (i, e) in espacos.iter().enumerate() {
            let dela: Vec<&Janela> = todas.iter().filter(|j| j.espaco == Some(e.id)).collect();
            // niri keeps one empty workspace at the end: it is the "new workspace" tile
            if i == espacos.len() - 1 && dela.is_empty() && i > 0 {
                let novo = gtk::Button::new();
                novo.add_css_class("visao-novo-espaco");
                novo.set_size_request(WS_H as i32, WS_H as i32);
                novo.set_child(Some(&ui_icon("mais", "#8E8F99", 16)));
                novo.set_tooltip_text(Some(tr("New workspace")));
                let (w, id) = (Rc::downgrade(self), e.id);
                novo.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.ir_para_espaco(id);
                    }
                });
                self.alvos.borrow_mut().push((novo.clone().upcast(), e.id));
                self.espacos.append(&novo);
                continue;
            }
            let quadro = gtk::Overlay::new();
            quadro.set_size_request(WS_W as i32, WS_H as i32);
            quadro.add_css_class("visao-espaco");
            if e.id == self.espaco.get() {
                quadro.add_css_class("ativo");
            }
            quadro.set_overflow(gtk::Overflow::Hidden);
            let fixo = gtk::Fixed::new();
            if let Some(p) = &papel {
                fixo.put(&figura(p, WS_W as i32, WS_H as i32), 0.0, 0.0);
            }
            // the windows where they are on the workspace (those in view)
            let lugares = lugares_reais(&dela, ow);
            for j in dela.iter() {
                let Some(r) = lugares.get(&j.id) else { continue };
                if r.x >= ow || r.x + r.w <= 0.0 {
                    continue;
                }
                let (px, py) = (r.x, r.y);
                let (w, h) = ((r.w * k).max(4.0), (r.h * WS_H / oh).max(4.0));
                let mini: gtk::Widget = match self.texturas.borrow().get(&j.id) {
                    Some(t) => figura(t, w as i32, h as i32).upcast(),
                    None => {
                        let c = caixa(Orientation::Vertical, 0, "visao-mini");
                        c.set_size_request(w as i32, h as i32);
                        c.upcast()
                    }
                };
                mini.add_css_class("visao-mini-borda");
                fixo.put(&mini, px * k, py * WS_H / oh);
            }
            quadro.set_child(Some(&fixo));
            let chip = caixa(Orientation::Horizontal, 5, "visao-espaco-rotulo");
            chip.set_halign(Align::Start);
            chip.set_valign(Align::End);
            chip.set_margin_start(6);
            chip.set_margin_bottom(9);
            let ponto = caixa(Orientation::Vertical, 0, "visao-ponto");
            ponto.set_size_request(6, 6);
            ponto.set_valign(Align::Center);
            ponto.add_css_class(&format!("cor{}", (e.idx.max(1) - 1) % DOTS.len() as u64));
            chip.append(&ponto);
            let nome = e.nome.clone().unwrap_or_else(|| trf("Workspace {n}", &[("n", e.idx.to_string())]));
            let nome_rotulo = rotulo(&nome, "visao-espaco-nome");
            chip.append(&nome_rotulo);
            quadro.add_overlay(&chip);
            quadro.set_tooltip_text(Some(tr("Double click to rename")));
            let clique = gtk::GestureClick::new();
            let (w, id, c, l) = (Rc::downgrade(self), e.id, chip.clone(), nome_rotulo.clone());
            let atual = e.nome.clone().unwrap_or_default();
            clique.connect_released(move |g, n, _, _| {
                let Some(v) = w.upgrade() else { return };
                g.set_state(gtk::EventSequenceState::Claimed);
                if debug() {
                    eprintln!("debug visão geral: clique no espaço {id} (n = {n})");
                }
                if n >= 2 {
                    v.renomear(&c, &l, id, &atual);
                } else {
                    v.ir_para_espaco(id);
                }
            });
            quadro.add_controller(clique);
            self.alvos.borrow_mut().push((quadro.clone().upcast(), e.id));
            self.espacos.append(&quadro);
        }
        if debug() {
            self.registrar_espacos(10);
        }
    }

    /// Debug: where each workspace tile is (the session test clicks and drops on them), once they are laid out.
    fn registrar_espacos(self: &Rc<Self>, tentativas: u32) {
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(300), move || {
            let Some(v) = w.upgrade() else { return };
            let mut linhas = Vec::new();
            let mut c = v.espacos.first_child();
            while let Some(x) = c {
                match x.compute_bounds(&v.janela).filter(|b| x.width() > 0 && b.width() >= 100.0) {
                    Some(b) => linhas.push(format!("{} em {:.0} {:.0}", if x.is::<gtk::Button>() { "novo" } else { "existente" }, b.x() + b.width() / 2.0, b.y() + b.height() / 2.0)),
                    None => {
                        if tentativas > 0 && v.aberta.get() {
                            v.registrar_espacos(tentativas - 1);
                        }
                        return;
                    }
                }
                c = x.next_sibling();
            }
            for (i, l) in linhas.iter().enumerate() {
                eprintln!("debug visão geral: espaço #{i} {l}");
            }
        });
    }

    /// The workspace tile under this point of the window (where a dragged window would go).
    fn tile_em(&self, x: f64, y: f64) -> Option<(gtk::Widget, u64)> {
        self.alvos.borrow().iter().find(|(w, _)| {
            w.compute_bounds(&self.janela).is_some_and(|b| b.contains_point(&gtk::graphene::Point::new(x as f32, y as f32)))
        }).cloned()
    }

    fn ir_para_espaco(self: &Rc<Self>, id: u64) {
        if id == self.espaco.get() {
            return;
        }
        let antes = self.espaco.replace(id);
        let _ = niri_acao(json!({"FocusWorkspace": {"reference": {"Id": id}}}));
        // the windows slide in from the side of the workspace chosen
        let idx = |e: u64| self.shell.borrow().upgrade().and_then(|s| s.estado.espacos.borrow().iter().find(|x| x.id == e).map(|x| x.idx)).unwrap_or(0);
        let lado = if idx(id) > idx(antes) { 1.0 } else { -1.0 };
        self.assinatura.borrow_mut().clear();
        self.render(false);
        for (_, a, _, _) in self.cartoes.borrow().iter() {
            a.definir(Canal::X, 80.0 * lado).definir(Canal::Opacidade, 0.0);
            a.ir(Canal::X, 0.0, mv::SUAVE).ir(Canal::Opacidade, 1.0, mv::SUAVE);
        }
        if debug() {
            eprintln!("debug visão geral: espaço {id}");
        }
    }

    /// The workspace's name becomes a field in its label (an empty name gives the number back; Esc cancels).
    fn renomear(self: &Rc<Self>, chip: &gtk::Box, rotulo_nome: &gtk::Label, id: u64, atual: &str) {
        if self.renomeando.borrow().is_some() {
            return;
        }
        let e = gtk::Entry::new();
        e.add_css_class("visao-espaco-campo");
        e.set_text(atual);
        e.set_placeholder_text(Some(tr("Workspace name")));
        e.set_width_chars(12);
        rotulo_nome.set_visible(false);
        chip.append(&e);
        let (c, l, en) = (chip.clone(), rotulo_nome.clone(), e.clone());
        let w = Rc::downgrade(self);
        let fim = move || {
            c.remove(&en);
            l.set_visible(true);
            if let Some(v) = w.upgrade() {
                v.renomeando.borrow_mut().take();
                v.busca.grab_focus();
                v.assinatura.borrow_mut().clear();
                v.render(false);
            }
        };
        let fim = Rc::new(fim);
        let f = fim.clone();
        *self.renomeando.borrow_mut() = Some(Box::new(move || f()));
        let w = Rc::downgrade(self);
        e.connect_activate(move |e| {
            let nome = e.text().trim().to_string();
            let r = if nome.is_empty() {
                niri_acao(json!({"UnsetWorkspaceName": {"reference": {"Id": id}}}))
            } else {
                niri_acao(json!({"SetWorkspaceName": {"name": nome, "workspace": {"Id": id}}}))
            };
            if let Err(err) = r {
                eprintln!("kalango-shell: renomear o espaço: {err}");
            }
            if debug() {
                eprintln!("debug visão geral: espaço {id} renomeado para “{nome}”");
            }
            if let Some(v) = w.upgrade() {
                v.renomeando.borrow_mut().take();
            }
            fim();
        });
        // the field takes the keyboard once it is on screen (added to a shown label it is mapped at once; if not,
        // when it is)
        let focar = |e: &gtk::Entry| {
            let e = e.clone();
            glib::idle_add_local_once(move || {
                e.grab_focus();
            });
        };
        if e.is_mapped() {
            focar(&e);
        } else {
            e.connect_map(focar);
        }
        if debug() {
            eprintln!("debug visão geral: renomeando o espaço {id}");
        }
    }

    fn render_janelas(self: &Rc<Self>, s: &Rc<Shell>, abrindo: bool) {
        while let Some(c) = self.area.first_child() {
            self.area.remove(&c);
        }
        self.cartoes.borrow_mut().clear();
        let (ow, oh) = self.tela.get();
        // the area: x 120 … (the agents' panel or the right margin), y 262 … above the dock
        let com_painel = self.painel.is_visible();
        let (ax, ay) = (120.0, 262.0);
        let aw = ow - ax - if com_painel { 560.0 } else { 120.0 };
        let ah = oh - ay - 106.0;
        let janelas = self.janelas_do(self.espaco.get());
        self.vazio.set_visible(janelas.is_empty());
        let proporcoes: Vec<(f64, f64)> = janelas.iter().map(|j| {
            let (w, h) = (f64::from(j.tamanho.0.max(1)), f64::from(j.tamanho.1.max(1)));
            match self.texturas.borrow().get(&j.id) {
                Some(t) if t.height() > 0 => (f64::from(t.width()) / f64::from(t.height()), w.max(120.0)),
                _ => (w / h, w.max(120.0)),
            }
        }).collect();
        let lugares = arranjar(&proporcoes, aw, ah);
        let reais = lugares_reais(&janelas.iter().collect::<Vec<_>>(), ow);
        for (j, l) in janelas.iter().zip(lugares) {
            let l = Lugar { x: l.x + ax, y: l.y + ay, ..l };
            let card = caixa(Orientation::Vertical, 8, "");
            let quadro = gtk::Overlay::new();
            let moldura = caixa(Orientation::Vertical, 0, "visao-miniatura");
            if j.focada {
                moldura.add_css_class("focada");
            }
            moldura.set_size_request(l.w as i32, l.h as i32);
            moldura.set_overflow(gtk::Overflow::Hidden);
            let app = apps::da_janela(&j.app_id, &s.todos).cloned();
            match self.texturas.borrow().get(&j.id) {
                Some(t) => moldura.append(&figura(t, l.w as i32, l.h as i32)),
                None => {
                    if let Some(a) = &app {
                        let i = apps::imagem(a, 64);
                        i.set_vexpand(true);
                        i.set_valign(Align::Center);
                        moldura.append(&i);
                    }
                }
            }
            quadro.set_child(Some(&moldura));
            let fechar = gtk::Button::new();
            fechar.set_child(Some(&ui_icon("fechar", "#E2E2E6", 14)));
            fechar.add_css_class("visao-fechar");
            fechar.set_halign(Align::End);
            fechar.set_valign(Align::Start);
            fechar.set_margin_top(8);
            fechar.set_margin_end(8);
            fechar.set_opacity(0.0);
            fechar.set_tooltip_text(Some(tr("Close window")));
            let id = j.id;
            fechar.connect_clicked(move |_| {
                let _ = niri_acao(json!({"CloseWindow": {"id": id}}));
            });
            quadro.add_overlay(&fechar);
            let movimento = gtk::EventControllerMotion::new();
            let f = fechar.clone();
            movimento.connect_enter(move |_, _, _| f.set_opacity(1.0));
            let f = fechar.clone();
            movimento.connect_leave(move |_| f.set_opacity(0.0));
            quadro.add_controller(movimento);
            card.append(&quadro);
            let chip = caixa(Orientation::Horizontal, 6, "visao-rotulo");
            chip.set_halign(Align::Center);
            if let Some(a) = &app {
                chip.append(&apps::imagem(a, 18));
            }
            let nome = app.as_ref().map(|a| a.nome.clone()).unwrap_or_else(|| j.titulo.clone());
            let n = rotulo(&nome, "visao-rotulo-nome");
            n.set_ellipsize(gtk::pango::EllipsizeMode::End);
            n.set_max_width_chars(28);
            chip.append(&n);
            chip.set_tooltip_text(Some(&j.titulo));
            card.append(&chip);
            card.set_size_request(l.w as i32, -1);
            let anim = Animavel::new(&card);
            self.area.put(&anim, l.x - ax, l.y - ay);
            // one gesture: a click goes to the window (middle click closes it); dragging carries the picture onto
            // a workspace (the shell does it: compositors do not all deliver drag and drop to layer surfaces)
            let arrasto = gtk::GestureDrag::new();
            arrasto.set_button(0);
            let movido = Rc::new(Cell::new(false));
            let alvo: Rc<RefCell<Option<gtk::Widget>>> = Rc::default();
            let (w, a, m, al, mo) = (Rc::downgrade(self), anim.clone(), movido.clone(), alvo.clone(), moldura.clone());
            arrasto.connect_drag_update(move |g, dx, dy| {
                let Some(v) = w.upgrade() else { return };
                if !m.get() && dx.hypot(dy) < 8.0 {
                    return;
                }
                if !m.replace(true) {
                    v.arrastando.set(true);
                    a.definir(Canal::Escala, 0.6).definir(Canal::Opacidade, 0.9);
                }
                a.definir(Canal::X, dx).definir(Canal::Y, dy);
                let (sx, sy) = g.start_point().unwrap_or_default();
                let p = mo.compute_point(&v.janela, &gtk::graphene::Point::new((sx + dx) as f32, (sy + dy) as f32));
                let novo = p.and_then(|p| v.tile_em(f64::from(p.x()), f64::from(p.y()))).map(|t| t.0);
                if *al.borrow() != novo {
                    if let Some(o) = al.borrow().as_ref() {
                        o.remove_css_class("soltar");
                    }
                    if let Some(n) = &novo {
                        n.add_css_class("soltar");
                    }
                    *al.borrow_mut() = novo;
                }
            });
            let (w, a, m, al, mo) = (Rc::downgrade(self), anim.clone(), movido.clone(), alvo.clone(), moldura.clone());
            arrasto.connect_drag_end(move |g, dx, dy| {
                let Some(v) = w.upgrade() else { return };
                g.set_state(gtk::EventSequenceState::Claimed);
                if !m.replace(false) {
                    if g.current_button() == 2 {
                        let _ = niri_acao(json!({"CloseWindow": {"id": id}}));
                    } else {
                        if debug() {
                            eprintln!("debug visão geral: ir para a janela {id}");
                        }
                        v.fechar(Some(id));
                    }
                    return;
                }
                v.arrastando.set(false);
                if let Some(o) = al.borrow_mut().take() {
                    o.remove_css_class("soltar");
                }
                let (sx, sy) = g.start_point().unwrap_or_default();
                let p = mo.compute_point(&v.janela, &gtk::graphene::Point::new((sx + dx) as f32, (sy + dy) as f32));
                match p.and_then(|p| v.tile_em(f64::from(p.x()), f64::from(p.y()))) {
                    Some((_, espaco)) if espaco != v.espaco.get() => {
                        if debug() {
                            eprintln!("debug visão geral: janela {id} para o espaço {espaco}");
                        }
                        if let Err(e) = niri_acao(json!({"MoveWindowToWorkspace": {"window_id": id, "reference": {"Id": espaco}, "focus": false}})) {
                            eprintln!("kalango-shell: mover a janela: {e}");
                        }
                        a.ir(Canal::Opacidade, 0.0, mv::RAPIDA);
                        v.assinatura.borrow_mut().clear();
                    }
                    _ => {
                        a.ir(Canal::X, 0.0, mv::SUAVE).ir(Canal::Y, 0.0, mv::SUAVE).ir(Canal::Escala, 1.0, mv::SUAVE).ir(Canal::Opacidade, 1.0, mv::SUAVE);
                    }
                }
                // redrawn after this event: the gesture's own widget must outlive its handler
                let w = Rc::downgrade(&v);
                glib::idle_add_local_once(move || {
                    if let Some(v) = w.upgrade() {
                        v.render(false);
                    }
                });
            });
            moldura.add_controller(arrasto);
            // where the window is now (the picture starts there when the overview opens)
            let real = reais.get(&j.id).copied().filter(|r| r.x < ow && r.x + r.w > 0.0);
            if abrindo {
                match real {
                    Some(r) => anim.entrar(((r.x + r.w / 2.0) - (l.x + l.w / 2.0), (r.y + r.h / 2.0) - (l.y + l.h / 2.0)), r.w / l.w),
                    None => anim.entrar((0.0, 12.0), 0.96),
                }
            }
            self.cartoes.borrow_mut().push((j.id, anim, l, real));
        }
        if debug() {
            eprintln!("debug visão geral: {} janelas: {}", janelas.len(),
                self.cartoes.borrow().iter().map(|(id, _, l, _)| format!("{id}@{:.0},{:.0} {:.0}x{:.0}", l.x, l.y, l.w, l.h)).collect::<Vec<_>>().join(" "));
        }
    }

    fn atualizar_pendentes(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(|| servicos::kalangod_pendentes().unwrap_or_default(), move |p| {
            if let Some(v) = w.upgrade() {
                *v.pendentes.borrow_mut() = p;
                v.render_agentes();
            }
        });
    }

    /// "Agentes trabalhando": the agent of the side bar while it works, and the requests waiting for the person.
    fn render_agentes(self: &Rc<Self>) {
        let Some(s) = self.shell.borrow().upgrade() else { return };
        let conversa = s.right_bar().conversa.clone();
        let tarefa = s.estado.agent_task.borrow().clone();
        let trabalho = conversa.work();
        let agora = glib::real_time() / 1_000_000;
        let pendentes = self.pendentes.borrow().clone();
        // every 2 s, the requests again (in the background)
        if agora % 2 == 0 && self.aberta.get() {
            self.atualizar_pendentes();
        }
        let assinatura = format!("{:?}|{:?}|{:?}", tarefa, trabalho.map(|(t, k, p)| (t.elapsed().as_secs() / 60, k / 1000, p)),
            pendentes.iter().map(|p| (p["id"].as_u64(), (agora as u64).saturating_sub(p["criado"].as_u64().unwrap_or(0)) / 10)).collect::<Vec<_>>());
        if *self.assinatura_agentes.borrow() == assinatura {
            return;
        }
        *self.assinatura_agentes.borrow_mut() = assinatura;
        let tinha = self.painel.is_visible();
        while let Some(c) = self.agentes.first_child() {
            self.agentes.remove(&c);
        }
        let titulo = caixa(Orientation::Horizontal, 6, "");
        titulo.append(&ui_icon("spark", "#A5A1E2", 14));
        titulo.append(&rotulo(tr("AGENTS AT WORK"), "caps"));
        self.agentes.append(&titulo);
        let mut cartoes = 0;
        if let (Some((pedido, passos)), Some((inicio, tokens, pausado))) = (tarefa, trabalho) {
            let estado = if pausado { (tr("Paused"), "pausado") } else { (tr("Running"), "executando") };
            let meta = format!("{} · {} · {}",
                if passos == 1 { tr("1 step").to_string() } else { trf("{n} steps", &[("n", passos.to_string())]) },
                minutos(inicio.elapsed().as_secs()),
                if tokens > 0 { tokens_text(tokens) } else { tr("starting").to_string() });
            let c = self.cartao_tarefa(tr("Agent"), tr("local agent"), estado, &pedido, (passos as f64 / MAX_STEPS).min(0.95), &meta);
            let botoes = c.last_child().and_then(|m| m.downcast::<gtk::Box>().ok());
            if let Some(b) = botoes {
                let pausa = self.botao(if pausado { "tocar" } else { "pausa" }, if pausado { tr("Resume") } else { tr("Pause") });
                let conv = conversa.clone();
                let w = Rc::downgrade(self);
                pausa.connect_clicked(move |_| {
                    conv.pause(!pausado);
                    if let Some(v) = w.upgrade() {
                        v.assinatura_agentes.borrow_mut().clear();
                        v.render_agentes();
                    }
                });
                b.append(&pausa);
                let parar = self.botao("parar", tr("Stop"));
                let conv = conversa.clone();
                parar.connect_clicked(move |_| {
                    conv.pause(false);
                    conv.stop(true);
                });
                b.append(&parar);
            }
            self.agentes.append(&c);
            cartoes += 1;
        }
        for p in pendentes.iter().take(3 - cartoes) {
            // who asked: the program, as the kernel reported it to kalangod
            let exe = p["exe"].as_str().and_then(|e| std::path::Path::new(e).file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (nome, sub) = if exe == "kalango-agente" {
                (tr("Agent").to_string(), tr("local agent").to_string())
            } else if exe.is_empty() {
                (p["caller"].as_str().unwrap_or("").to_string(), tr("program").to_string())
            } else {
                (exe, tr("program").to_string())
            };
            let desde = (agora as u64).saturating_sub(p["criado"].as_u64().unwrap_or(agora as u64));
            let prazo = p["prazo_s"].as_u64().unwrap_or(120).max(1);
            let descricao = p["descricao"].as_str().filter(|d| !d.is_empty()).map(|d| d.split(['.', '\n']).next().unwrap_or(d).to_string())
                .unwrap_or_else(|| p["tool"].as_str().unwrap_or("").to_string());
            let meta = trf("waiting for approval for {t}", &[("t", minutos(desde))]);
            let c = self.cartao_tarefa(&nome, &sub, (tr("Waiting"), "aguardando"), &descricao, desde as f64 / prazo as f64, &meta);
            if let Some(b) = c.last_child().and_then(|m| m.downcast::<gtk::Box>().ok()) {
                // seeing (and approving) goes through the side bar and the approval screen; refusing is safe here
                let ver = self.botao("olho", tr("See the request"));
                let w = Rc::downgrade(self);
                ver.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.fechar(None);
                        if let Some(s) = v.shell.borrow().upgrade() {
                            s.right_bar().abrir(crate::lateral::Modo::Agente);
                        }
                    }
                });
                b.append(&ver);
                let recusar = self.botao("parar", tr("Refuse"));
                let id = p["id"].as_u64().unwrap_or(0);
                let w = Rc::downgrade(self);
                recusar.connect_clicked(move |_| {
                    if let Err(e) = servicos::kalangod_decidir(id, false) {
                        eprintln!("kalango-shell: recusar o pedido {id}: {e}");
                    }
                    if let Some(v) = w.upgrade() {
                        v.atualizar_pendentes();
                    }
                });
                b.append(&recusar);
            }
            self.agentes.append(&c);
            cartoes += 1;
        }
        let central = gtk::Button::with_label(tr("Open the agents center"));
        central.add_css_class("visao-central");
        central.set_halign(Align::Start);
        let w = Rc::downgrade(self);
        central.connect_clicked(move |_| {
            if let Some(v) = w.upgrade() {
                v.fechar(None);
                let shell = v.shell.borrow().upgrade();
                if let Some(s) = shell {
                    s.abrir_central(None);
                }
            }
        });
        self.agentes.append(&central);
        let mostrar = cartoes > 0;
        self.painel.set_visible(mostrar);
        if mostrar && !tinha {
            self.painel.definir(Canal::Opacidade, 1.0);
            self.painel.entrar((40.0, 0.0), 1.0);
        }
        if mostrar != tinha {
            // the windows' area depends on the panel
            self.assinatura.borrow_mut().clear();
            self.render(false);
        }
        if debug() {
            eprintln!("debug visão geral: {cartoes} cartões de agentes");
        }
    }

    fn botao(&self, glifo: &str, dica: &str) -> gtk::Button {
        let b = gtk::Button::new();
        b.set_child(Some(&ui_icon(glifo, "#E2E2E6", 14)));
        b.add_css_class("visao-tarefa-botao");
        b.set_tooltip_text(Some(dica));
        b.set_widget_name(dica);
        b
    }

    /// A task card; its last child is the meta row, where the buttons go.
    fn cartao_tarefa(&self, nome: &str, sub: &str, estado: (&str, &str), titulo: &str, progresso: f64, meta: &str) -> gtk::Box {
        let c = caixa(Orientation::Vertical, 10, "visao-tarefa");
        let cab = caixa(Orientation::Horizontal, 8, "");
        let av = caixa(Orientation::Horizontal, 0, "visao-tarefa-avatar");
        av.set_size_request(28, 28);
        av.set_valign(Align::Center);
        av.set_hexpand(false);
        let sp = ui_icon("spark", "#A5A1E2", 14);
        sp.set_hexpand(true);
        sp.set_halign(Align::Center);
        sp.set_valign(Align::Center);
        av.append(&sp);
        cab.append(&av);
        let col = caixa(Orientation::Vertical, 1, "");
        col.set_hexpand(true);
        let n = rotulo(nome, "visao-tarefa-nome");
        n.set_xalign(0.0);
        col.append(&n);
        let s = rotulo(sub, "visao-tarefa-sub");
        s.set_xalign(0.0);
        col.append(&s);
        cab.append(&col);
        let chip = rotulo(estado.0, "visao-tarefa-estado");
        chip.add_css_class(estado.1);
        chip.set_valign(Align::Center);
        cab.append(&chip);
        c.append(&cab);
        let t = rotulo(titulo, "visao-tarefa-titulo");
        t.set_xalign(0.0);
        t.set_ellipsize(gtk::pango::EllipsizeMode::End);
        t.set_max_width_chars(1);
        t.set_hexpand(true);
        c.append(&t);
        let barra = caixa(Orientation::Horizontal, 0, "visao-progresso");
        barra.set_size_request(338, 4);
        let valor = caixa(Orientation::Horizontal, 0, "visao-progresso-valor");
        valor.add_css_class(estado.1);
        valor.set_size_request((338.0 * progresso.clamp(0.02, 1.0)) as i32, 4);
        valor.set_halign(Align::Start);
        barra.append(&valor);
        c.append(&barra);
        let linha = caixa(Orientation::Horizontal, 8, "");
        let m = rotulo(meta, "visao-tarefa-meta");
        m.set_xalign(0.0);
        m.set_hexpand(true);
        m.set_ellipsize(gtk::pango::EllipsizeMode::End);
        m.set_max_width_chars(1);
        linha.append(&m);
        c.append(&linha);
        c
    }
}

#[cfg(test)]
mod tests {
    use super::arranjar;

    /// Three windows like the design's (a wide one and two smaller): every picture fits the area, none overlaps,
    /// none is bigger than its window, and the first one gets the most room.
    #[test]
    fn lays_out_windows_without_overlap() {
        let js = [(1.61, 1400.0), (1.59, 900.0), (1.59, 900.0)];
        let l = arranjar(&js, 1240.0, 712.0);
        assert_eq!(l.len(), 3);
        for (i, a) in l.iter().enumerate() {
            assert!(a.x >= -0.5 && a.y >= -0.5 && a.x + a.w <= 1240.5 && a.y + a.h + super::LABEL <= 712.5, "{a:?}");
            assert!(a.w <= js[i].1 + 0.5);
            for b in l.iter().skip(i + 1) {
                let separadas = a.x + a.w <= b.x + 0.5 || b.x + b.w <= a.x + 0.5 || a.y + a.h + super::LABEL <= b.y + 0.5 || b.y + b.h + super::LABEL <= a.y + 0.5;
                assert!(separadas, "{a:?} {b:?}");
            }
        }
        assert!(l[0].w * l[0].h >= l[1].w * l[1].h);
        // one small window is shown at its own size, not blown up
        let um = arranjar(&[(1.5, 300.0)], 1240.0, 712.0);
        assert!((um[0].w - 300.0).abs() < 0.5);
    }
}
