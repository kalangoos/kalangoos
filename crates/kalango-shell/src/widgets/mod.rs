//! Desktop widgets (design "04 · Área de trabalho" and "Widgets"): on the bottom layer, above the wallpaper and below
//! the windows. Every widget exists in S 168×168, M 352×168 and/or L 352×352, on a grid of 184 px (168 + 16); each
//! one is anchored to the left or the right edge, so the layout follows the screen's width (and a side bar pinned on
//! the right). The layout is the person's (~/.config/kalango/widgets.json); without one, the design's default.
//!
//! - On a short screen (under 800 px, the 1366×768 notebook) or one too narrow for both columns, only the clock.
//! - A widget that does not fit above the dock is not shown.
//! - Transparent by default; when the wallpaper under it gives the text less than 4.5:1 of contrast, the widget gets a
//!   base of the theme's colour at 60% by itself.
//!
//! Motion: the widgets come in with the gentle spring, in a 24 ms cascade.

pub(crate) mod comum;
pub(crate) mod dados;
mod geral;
mod edicao;
pub(crate) mod ia;
mod metricas;

pub use comum::quadro;
use crate::superficies::{camada, papel_de_parede, BARRA_ALTURA};
use crate::Shell;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::n_;
use kalango_movimento::{self as mv, Animavel, Canal};
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

/// The grid: a 168 px cell and 16 px between cells.
pub const CELULA: i32 = 168;
pub const PASSO: i32 = 184;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tam {
    P,
    M,
    G,
}

impl Tam {
    pub fn px(self) -> (i32, i32) {
        match self {
            Tam::P => (CELULA, CELULA),
            Tam::M => (CELULA * 2 + 16, CELULA),
            Tam::G => (CELULA * 2 + 16, CELULA * 2 + 16),
        }
    }
}

/// A kind of widget: its id, its name, its group in the gallery and the sizes it comes in.
pub struct Tipo {
    pub id: &'static str,
    pub nome: &'static str,
    pub grupo: &'static str,
    pub tamanhos: &'static [Tam],
}

pub const TIPOS: &[Tipo] = &[
    Tipo { id: "relogio", nome: n_("Clock"), grupo: "sistema", tamanhos: &[Tam::P] },
    Tipo { id: "resumo", nome: n_("Day summary"), grupo: "ia", tamanhos: &[Tam::G] },
    // S: how many and who; M: what each one does; L: their last steps
    Tipo { id: "agentes-ativos", nome: n_("Active agents"), grupo: "ia", tamanhos: &[Tam::P, Tam::M, Tam::G] },
    Tipo { id: "pergunte", nome: n_("Ask KalangoOS"), grupo: "ia", tamanhos: &[Tam::M] },
    Tipo { id: "agora", nome: n_("Now"), grupo: "ia", tamanhos: &[Tam::M] },
    Tipo { id: "tokens", nome: n_("Tokens today"), grupo: "ia", tamanhos: &[Tam::P] },
    Tipo { id: "modelo", nome: n_("Local model"), grupo: "ia", tamanhos: &[Tam::P] },
    Tipo { id: "privacidade", nome: n_("Privacy"), grupo: "ia", tamanhos: &[Tam::P] },
    Tipo { id: "cpu", nome: n_("CPU"), grupo: "metricas", tamanhos: &[Tam::P] },
    Tipo { id: "gpu", nome: n_("GPU"), grupo: "metricas", tamanhos: &[Tam::P] },
    Tipo { id: "memoria", nome: n_("Memory"), grupo: "metricas", tamanhos: &[Tam::P] },
    Tipo { id: "rede", nome: n_("Network"), grupo: "metricas", tamanhos: &[Tam::M] },
    Tipo { id: "armazenamento", nome: n_("Storage"), grupo: "metricas", tamanhos: &[Tam::M] },
    Tipo { id: "sistema", nome: n_("System"), grupo: "metricas", tamanhos: &[Tam::G] },
    Tipo { id: "hoje", nome: n_("Today"), grupo: "sistema", tamanhos: &[Tam::M] },
    Tipo { id: "tocando", nome: n_("Now playing"), grupo: "sistema", tamanhos: &[Tam::M] },
    Tipo { id: "clima", nome: n_("Weather"), grupo: "sistema", tamanhos: &[Tam::P] },
    Tipo { id: "foco", nome: n_("Focus"), grupo: "sistema", tamanhos: &[Tam::P] },
    Tipo { id: "dispositivos", nome: n_("Devices"), grupo: "sistema", tamanhos: &[Tam::M] },
    Tipo { id: "nota", nome: n_("Quick note"), grupo: "sistema", tamanhos: &[Tam::M] },
    Tipo { id: "acoes", nome: n_("Quick actions"), grupo: "sistema", tamanhos: &[Tam::M] },
    Tipo { id: "calendario", nome: n_("Calendar"), grupo: "sistema", tamanhos: &[Tam::G] },
];

/// One widget on the desktop: `x` is the distance from its anchor edge to its near side, `y` from the top of the
/// screen (logical pixels).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Colocado {
    pub tipo: String,
    pub tam: Tam,
    pub x: i32,
    pub y: i32,
    pub direita: bool,
}

impl Colocado {
    fn novo(tipo: &str, tam: Tam, x: i32, y: i32, direita: bool) -> Colocado {
        Colocado { tipo: tipo.into(), tam, x, y, direita }
    }
}

/// The design's desktop ("Área de trabalho", Full HD).
pub fn padrao() -> Vec<Colocado> {
    vec![
        Colocado::novo("relogio", Tam::P, 48, 64, false),
        Colocado::novo("resumo", Tam::G, 48, 300, false),
        Colocado::novo("agentes-ativos", Tam::M, 48, 72, true),
        Colocado::novo("pergunte", Tam::M, 48, 256, true),
        Colocado::novo("agora", Tam::M, 48, 440, true),
    ]
}

fn arquivo() -> PathBuf {
    glib::user_config_dir().join("kalango/widgets.json")
}

/// The person's layout (or the design's when there is none, or it cannot be read).
pub fn carregar() -> Vec<Colocado> {
    std::fs::read(arquivo()).ok().and_then(|b| serde_json::from_slice::<Vec<Colocado>>(&b).ok())
        .map(|v| v.into_iter().filter(|c| TIPOS.iter().any(|t| t.id == c.tipo && t.tamanhos.contains(&c.tam))).collect())
        .unwrap_or_else(padrao)
}

pub fn salvar(v: &[Colocado]) -> std::io::Result<()> {
    let a = arquivo();
    if let Some(d) = a.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = a.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v).unwrap_or_default())?;
    std::fs::rename(tmp, a)
}

/// "Resumir esta tela": a picture of the window in front (niri's thumbnail: nothing goes to the clipboard), or of the
/// screen when no window has the focus, described by the local vision model; the request and the answer show in the
/// Agent bar. The description is only read by the person: it never drives the agent's tools.
pub fn visao_da_tela(shell: &Rc<Shell>) {
    use kalango_i18n::tr;
    let rb = shell.right_bar();
    rb.abrir(crate::lateral::Modo::Agente);
    let conv = rb.conversa.clone();
    conv.bolha_pessoa(tr("Summarize this screen"));
    conv.nota(tr("Looking at the screen with the local vision model…"));
    let path = crate::alternador::thumbs_dir().join("tela.png");
    let _ = std::fs::remove_file(&path);
    let foco = shell.estado.janelas.borrow().values().find(|j| j.focada).map(|j| j.id);
    let pedido = match foco {
        Some(id) => crate::servicos::niri_acao(serde_json::json!({"ThumbnailWindow": {"id": id, "path": path.display().to_string(), "max_size": 1600}})),
        None => crate::servicos::niri_acao(serde_json::json!({"ScreenshotScreen": {"write_to_disk": true, "show_pointer": false, "path": path.display().to_string()}})),
    };
    if let Err(e) = pedido {
        conv.nota(&kalango_i18n::trf("Could not take the picture: {error}", &[("error", e)]));
        return;
    }
    let inicio = std::time::Instant::now();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if !path.is_file() {
            if inicio.elapsed() > Duration::from_secs(5) {
                conv.nota(tr("Could not take the picture of the screen."));
                return glib::ControlFlow::Break;
            }
            return glib::ControlFlow::Continue;
        }
        let (conv, p) = (conv.clone(), path.clone());
        crate::lateral::em_segundo_plano(move || {
            let bin = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("kalango-agente"))).filter(|b| b.is_file())
                .unwrap_or_else(|| "kalango-agente".into());
            let r = std::process::Command::new(bin).arg("--json").arg("ver").arg(&p).output();
            let _ = std::fs::remove_file(&p);
            match r {
                Ok(o) if o.status.success() => serde_json::from_slice::<serde_json::Value>(&o.stdout).ok()
                    .and_then(|v| v["texto"].as_str().map(str::to_string)).ok_or_else(|| String::from_utf8_lossy(&o.stderr).into_owned()),
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("").to_string()),
                Err(e) => Err(e.to_string()),
            }
        }, move |r| match r {
            Ok(t) => conv.resposta(&t),
            Err(e) => conv.nota(&kalango_i18n::trf("The vision model could not answer: {error}", &[("error", e)])),
        });
        glib::ControlFlow::Break
    });
}

/// WCAG contrast between two relative luminances.
fn contraste(a: f64, b: f64) -> f64 {
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

fn luminancia(r: f64, g: f64, b: f64) -> f64 {
    let c = |v: f64| if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * c(r) + 0.7152 * c(g) + 0.0722 * c(b)
}

/// The mean luminance of the wallpaper under a rectangle (sampled every 6 px).
fn luminancia_do_papel(w: i32, h: i32, x: i32, y: i32, rw: i32, rh: i32) -> Option<f64> {
    let pb = papel_de_parede(w, h)?;
    let (pw, ph, rs, n) = (pb.width(), pb.height(), pb.rowstride() as usize, pb.n_channels() as usize);
    let px = pb.read_pixel_bytes();
    let (mut soma, mut conta) = (0.0, 0.0);
    for yy in (y.max(0)..(y + rh).min(ph)).step_by(6) {
        for xx in (x.max(0)..(x + rw).min(pw)).step_by(6) {
            let i = yy as usize * rs + xx as usize * n;
            if let Some(p) = px.get(i..i + 3) {
                soma += luminancia(f64::from(p[0]) / 255.0, f64::from(p[1]) / 255.0, f64::from(p[2]) / 255.0);
                conta += 1.0;
            }
        }
    }
    (conta > 0.0).then(|| soma / conta)
}

/// The margin from the screen's edges and the top of the grid.
const MARGEM: i32 = 48;
const TOPO: i32 = 64;
/// What the dock and the space around it take at the bottom.
const BASE: i32 = 62 + 16;

/// A widget's size on screen (the clock is as tall as its digits).
pub fn medida(c: &Colocado) -> (i32, i32) {
    if c.tipo == "relogio" { (CELULA, 188) } else { c.tam.px() }
}

/// Where it is on a screen `sw` wide: x, y, width, height (y from the top of the screen).
pub fn retangulo(c: &Colocado, sw: i32) -> (i32, i32, i32, i32) {
    let (w, h) = medida(c);
    (if c.direita { sw - c.x - w } else { c.x }, c.y, w, h)
}

fn sobrepoe(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// A dropped widget, at the grid's nearest cell: anchored to the edge nearest its middle.
pub fn encaixar(tipo: &str, tam: Tam, px: i32, py: i32, sw: i32) -> Colocado {
    let c = Colocado::novo(tipo, tam, 0, 0, false);
    let (w, _) = medida(&c);
    let direita = px + w / 2 > sw / 2;
    let bruto = if direita { sw - (px + w) } else { px };
    let x = MARGEM + (((bruto - MARGEM) as f64 / PASSO as f64).round() as i32).max(0) * PASSO;
    let y = TOPO + (((py - TOPO) as f64 / PASSO as f64).round() as i32).max(0) * PASSO;
    Colocado::novo(tipo, tam, x, y, direita)
}

/// Does `c` fit on this screen without covering another widget of `layout` (`exceto`: the one being moved)?
pub fn cabe(c: &Colocado, layout: &[Colocado], exceto: Option<usize>, sw: i32, sh: i32) -> bool {
    let r = retangulo(c, sw);
    r.0 >= 0 && r.0 + r.2 <= sw && r.1 >= TOPO.min(c.y) && r.1 + r.3 <= sh - BASE
        && layout.iter().enumerate().all(|(i, o)| Some(i) == exceto || !sobrepoe(r, retangulo(o, sw)))
}

/// The first free cell for a widget: the left side first, row by row, then the right side.
pub fn lugar_livre(tipo: &str, tam: Tam, layout: &[Colocado], sw: i32, sh: i32) -> Option<Colocado> {
    for direita in [false, true] {
        for col in 0..((sw / 2 - MARGEM) / PASSO).max(1) {
            for row in 0..((sh - TOPO) / PASSO).max(1) {
                let c = Colocado::novo(tipo, tam, MARGEM + col * PASSO, TOPO + row * PASSO, direita);
                if cabe(&c, layout, None, sw, sh) {
                    return Some(c);
                }
            }
        }
    }
    None
}

/// A whole desktop from a list of widgets (in order of importance): each one in the first free cell. Those that do
/// not fit are left out.
pub fn dispor(escolhidos: &[(String, Tam)], sw: i32, sh: i32) -> Vec<Colocado> {
    let mut v: Vec<Colocado> = Vec::new();
    for (t, tam) in escolhidos {
        if let Some(c) = lugar_livre(t, *tam, &v, sw, sh) {
            v.push(c);
        }
    }
    v
}

/// One widget on the desktop, in its frame: the outline, "−" and the handle show while editing.
struct Item {
    /// where it is: it changes in place when it is dragged (the buttons and gestures keep pointing to this item)
    c: RefCell<Colocado>,
    anim: Animavel,
    moldura: gtk::Overlay,
    widget: RefCell<gtk::Widget>,
    contorno: gtk::Box,
    remover: gtk::Button,
    alca: gtk::Box,
}

/// The gallery (400) with its margin and a gap: the right column's offset while editing.
const GALLERY_ROOM: i32 = 432;

/// The frame around a widget: 14 px on each side (the outline is 6 px out, "−" sticks out of its corner).
const FOLGA: i32 = 14;

pub struct Area {
    janela: gtk::ApplicationWindow,
    fundo: gtk::DrawingArea,
    fixo: gtk::Fixed,
    guias: gtk::Fixed,
    guia_v: gtk::Box,
    guia_h: gtk::Box,
    medida: gtk::Label,
    shell: RefCell<Weak<Shell>>,
    itens: RefCell<Vec<Rc<Item>>>,
    layout: RefCell<Vec<Colocado>>,
    /// the surface's size (it starts under the bar: y on screen = y here + the bar)
    tamanho: Cell<(i32, i32)>,
    /// the ask field, for the session test (where it is on screen)
    pergunta: RefCell<Option<gtk::Widget>>,
    pub editando: Cell<bool>,
    galeria: RefCell<Option<Rc<edicao::Galeria>>>,
    /// while editing, the room the gallery takes at the right: the right column moves left by it, so no widget is
    /// under the gallery (each keeps its distance to the right edge when saved)
    gallery_room: Cell<i32>,
}

impl Area {
    pub fn new(app: &gtk::Application, shell: &Rc<Shell>) -> Rc<Area> {
        let janela = camada(app, "kalango-area-de-trabalho", Layer::Bottom, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
        // the ask field takes the keyboard when clicked
        janela.set_keyboard_mode(KeyboardMode::OnDemand);
        janela.add_css_class("area-de-trabalho");
        let raiz = gtk::Overlay::new();
        let fundo = gtk::DrawingArea::new();
        fundo.set_hexpand(true);
        fundo.set_vexpand(true);
        raiz.set_child(Some(&fundo));
        let fixo = gtk::Fixed::new();
        raiz.add_overlay(&fixo);
        let guias = gtk::Fixed::new();
        guias.set_can_target(false);
        let guia_v = crate::superficies::caixa(gtk::Orientation::Vertical, 0, "widget-guia");
        let guia_h = crate::superficies::caixa(gtk::Orientation::Vertical, 0, "widget-guia");
        let medida_l = crate::superficies::rotulo("", "widget-medida");
        for g in [guia_v.upcast_ref::<gtk::Widget>(), guia_h.upcast_ref(), medida_l.upcast_ref()] {
            g.set_visible(false);
            guias.put(g, 0.0, 0.0);
        }
        raiz.add_overlay(&guias);
        janela.set_child(Some(&raiz));
        let a = Rc::new(Area {
            janela, fundo, fixo, guias, guia_v, guia_h, medida: medida_l, shell: RefCell::new(Rc::downgrade(shell)), itens: RefCell::default(),
            layout: RefCell::new(carregar()), tamanho: Cell::new((0, 0)), pergunta: RefCell::default(), editando: Cell::new(false),
            galeria: RefCell::default(), gallery_room: Cell::new(0),
        });
        // while editing: the wallpaper (the windows stay behind) and the scrim at 35%
        let w = Rc::downgrade(&a);
        a.fundo.set_draw_func(move |_, cr, _, _| {
            let Some(a) = w.upgrade().filter(|a| a.editando.get()) else { return };
            let (mw, mh) = a.monitor();
            if let Some(pb) = papel_de_parede(mw, mh) {
                use gtk::gdk::prelude::GdkCairoContextExt;
                cr.set_source_pixbuf(&pb, 0.0, -f64::from(BARRA_ALTURA));
                let _ = cr.paint();
            }
            cr.set_source_rgba(11.0 / 255.0, 12.0 / 255.0, 16.0 / 255.0, 0.35);
            let _ = cr.paint();
        });
        // the galery and the edit bar live on the same surface (above the windows while editing)
        let g = edicao::Galeria::new(&a);
        raiz.add_overlay(&g.painel);
        raiz.add_overlay(&g.barra);
        *a.galeria.borrow_mut() = Some(g);
        // a right click on the empty desktop: "Editar widgets"
        let menu = gtk::GestureClick::new();
        menu.set_button(3);
        let w = Rc::downgrade(&a);
        menu.connect_pressed(move |g, _, x, y| {
            let Some(a) = w.upgrade() else { return };
            if a.editando.get() || a.fixo.pick(x, y, gtk::PickFlags::DEFAULT).is_some_and(|p| p != a.fixo.clone().upcast::<gtk::Widget>()) {
                return;
            }
            g.set_state(gtk::EventSequenceState::Claimed);
            a.menu_da_area(x, y);
        });
        a.fixo.add_controller(menu);
        // Esc leaves editing (keeping what was done)
        let teclas = gtk::EventControllerKey::new();
        let w = Rc::downgrade(&a);
        teclas.connect_key_pressed(move |_, k, _, _| {
            let Some(a) = w.upgrade().filter(|a| a.editando.get()) else { return glib::Propagation::Proceed };
            if k == gdk::Key::Escape {
                a.sair_da_edicao();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        a.janela.add_controller(teclas);
        // the surface's size comes from the compositor (and changes with the screen or a pinned side bar)
        let w = Rc::downgrade(&a);
        a.janela.connect_realize(move |j| {
            let Some(s) = j.surface() else { return };
            let w2 = w.clone();
            let mudou = move |s: &gdk::Surface| {
                if let Some(a) = w2.upgrade() {
                    if s.width() > 0 && (s.width(), s.height()) != a.tamanho.get() {
                        a.tamanho.set((s.width(), s.height()));
                        a.posicionar();
                    }
                }
            };
            let m = mudou.clone();
            s.connect_width_notify(move |s| m(s));
            let m = mudou.clone();
            s.connect_height_notify(move |s| m(s));
        });
        a.construir();
        a.janela.present();
        // the monitor's size decides the compact desktop
        if let Some(d) = gdk::Display::default() {
            let w = Rc::downgrade(&a);
            let seguir = move |ms: &gtk::gio::ListModel| {
                for m in (0..ms.n_items()).filter_map(|i| ms.item(i).and_downcast::<gdk::Monitor>()) {
                    let w = w.clone();
                    m.connect_geometry_notify(move |_| {
                        if let Some(a) = w.upgrade() {
                            a.posicionar();
                        }
                    });
                }
            };
            let ms = d.monitors();
            seguir(&ms);
            let w = Rc::downgrade(&a);
            ms.connect_items_changed(move |ms, _, _, _| {
                seguir(ms);
                if let Some(a) = w.upgrade() {
                    a.posicionar();
                }
            });
        }
        if debug() {
            for ms in [3000u64, 10000, 25000] {
                let w = Rc::downgrade(&a);
                glib::timeout_add_local_once(Duration::from_millis(ms), move || {
                    if let Some(a) = w.upgrade() {
                        a.registrar_pergunta();
                    }
                });
            }
        }
        a
    }

    fn shell(&self) -> Option<Rc<Shell>> {
        self.shell.borrow().upgrade()
    }

    fn monitor(&self) -> (i32, i32) {
        let (sw, sh) = self.tamanho.get();
        gdk::Display::default().and_then(|d| d.monitors().item(0)).and_downcast::<gdk::Monitor>().map(|m| m.geometry())
            .map(|g| (g.width(), g.height())).unwrap_or((sw, sh + BARRA_ALTURA))
    }

    /// The screen's width and the height down to the bottom of this surface, in screen coordinates.
    fn tela(&self) -> (i32, i32) {
        let (sw, sh) = self.tamanho.get();
        (sw - self.gallery_room.get(), sh + BARRA_ALTURA)
    }

    /// The gallery's room changes: the right column slides to its new place (gentle spring).
    fn set_gallery_room(&self, room: i32) {
        let old = self.gallery_room.replace(room);
        if old == room {
            return;
        }
        for it in self.itens.borrow().iter().filter(|it| it.c.borrow().direita) {
            // it starts where it was and slides
            it.anim.definir(Canal::X, f64::from(room - old)).ir(Canal::X, 0.0, mv::SUAVE);
        }
        self.posicionar();
    }

    /// The widget of one kind and size (None for a kind this build does not have). `previa`: for the gallery (the
    /// summary and "Agora" do not ask the model there).
    pub(crate) fn widget(&self, tipo: &str, tam: Tam, previa: bool) -> Option<gtk::Widget> {
        let s = self.shell()?;
        let w = match (tipo, tam) {
            ("relogio", _) => crate::area::relogio(),
            ("resumo", _) if previa => crate::area::resumo_previa(&s),
            ("resumo", _) => crate::area::resumo(&s),
            ("agentes-ativos", Tam::P) => ia::agentes(&s),
            ("agentes-ativos", Tam::M) => crate::area::agentes_ativos(&s),
            ("agentes-ativos", Tam::G) => ia::agentes_detalhe(&s),
            ("pergunte", _) => {
                let w = crate::area::pergunte(&s);
                if !previa {
                    *self.pergunta.borrow_mut() = Some(w.clone());
                }
                w
            }
            ("agora", _) if previa => crate::area::agora_previa(),
            ("agora", _) => crate::area::agora(&s),
            ("tokens", _) => ia::tokens(),
            ("modelo", _) => ia::modelo(),
            ("privacidade", _) => ia::privacidade(),
            ("cpu", _) => metricas::cpu(),
            ("gpu", _) => metricas::gpu(),
            ("memoria", _) => metricas::memoria(),
            ("rede", _) => metricas::rede(),
            ("armazenamento", _) => metricas::armazenamento(),
            ("sistema", _) => metricas::sistema(),
            ("hoje", _) => geral::hoje(),
            ("tocando", _) => geral::tocando(&s),
            ("clima", _) => geral::clima(&s),
            ("foco", _) => geral::foco(&s),
            ("dispositivos", _) => geral::dispositivos(),
            ("nota", _) => geral::nota(&s),
            ("acoes", _) => geral::acoes(&s),
            ("calendario", _) => geral::calendario(),
            _ => return None,
        };
        w.add_css_class("widget");
        Some(w)
    }

    fn item(self: &Rc<Self>, c: Colocado) -> Option<Rc<Item>> {
        let w = self.widget(&c.tipo, c.tam, false)?;
        w.set_margin_start(FOLGA);
        w.set_margin_end(FOLGA);
        w.set_margin_top(FOLGA);
        w.set_margin_bottom(FOLGA);
        let moldura = gtk::Overlay::new();
        // the frame has the widget's size even when the widget has nothing to show ("Agora" with no suggestion):
        // while editing, its place and outline stay
        let (mw, mh) = medida(&c);
        moldura.set_size_request(mw + 2 * FOLGA, mh + 2 * FOLGA);
        w.set_size_request(mw, mh);
        w.set_valign(gtk::Align::Start);
        moldura.set_child(Some(&w));
        let contorno = crate::superficies::caixa(gtk::Orientation::Vertical, 0, "widget-contorno");
        contorno.set_margin_start(FOLGA - 6);
        contorno.set_margin_end(FOLGA - 6);
        contorno.set_margin_top(FOLGA - 6);
        contorno.set_margin_bottom(FOLGA - 6);
        contorno.set_can_target(false);
        moldura.add_overlay(&contorno);
        let remover = gtk::Button::new();
        remover.add_css_class("widget-remover");
        remover.set_child(Some(&crate::superficies::rotulo("−", "widget-remover-sinal")));
        remover.set_halign(gtk::Align::Start);
        remover.set_valign(gtk::Align::Start);
        remover.set_tooltip_text(Some(kalango_i18n::tr("Remove")));
        moldura.add_overlay(&remover);
        let alca = crate::superficies::caixa(gtk::Orientation::Vertical, 0, "widget-alca");
        alca.set_size_request(12, 12);
        alca.set_halign(gtk::Align::End);
        alca.set_valign(gtk::Align::End);
        alca.set_margin_end(6);
        alca.set_margin_bottom(6);
        alca.set_tooltip_text(Some(kalango_i18n::tr("Drag to change the size")));
        moldura.add_overlay(&alca);
        let anim = Animavel::new(&moldura);
        self.fixo.put(&anim, 0.0, 0.0);
        let it = Rc::new(Item { c: RefCell::new(c), anim, moldura, widget: RefCell::new(w), contorno, remover, alca });
        self.ligar_item(&it);
        self.decorar(&it);
        Some(it)
    }

    /// Builds every widget of the layout (again).
    pub fn construir(self: &Rc<Self>) {
        for it in self.itens.borrow_mut().drain(..) {
            self.fixo.remove(&it.anim);
        }
        let layout = self.layout.borrow().clone();
        let mut anims = Vec::new();
        for c in layout {
            if let Some(it) = self.item(c) {
                anims.push(it.anim.clone());
                self.itens.borrow_mut().push(it);
            }
        }
        self.posicionar();
        mv::cascata(&anims, 10.0);
    }

    /// The layout as it is on the desktop now.
    fn guardar_layout(&self) {
        *self.layout.borrow_mut() = self.itens.borrow().iter().map(|i| i.c.borrow().clone()).collect();
    }

    /// Edit mode's decorations on or off for one widget.
    fn decorar(&self, it: &Item) {
        let e = self.editando.get();
        it.contorno.set_visible(e);
        it.remover.set_visible(e);
        let tamanhos = TIPOS.iter().find(|t| t.id == it.c.borrow().tipo).map(|t| t.tamanhos.len()).unwrap_or(1);
        it.alca.set_visible(e && tamanhos > 1);
        // while editing, the widget itself takes no clicks: the frame is what moves
        it.widget.borrow().set_can_target(!e);
    }

    /// Where each widget goes on this screen, which ones fit, and which need a base for contrast.
    pub fn posicionar(&self) {
        let (sw, sh) = self.tamanho.get();
        let sw = sw - self.gallery_room.get();
        if sw == 0 {
            return;
        }
        let (mw, mh) = self.monitor();
        // design "Área de trabalho · Notebook 1366×768": a short screen, or one too narrow for both columns, keeps
        // only the clock
        let compacta = mh < 800 || mw < 832;
        // text colour of the widgets (#E2E2E6)
        let texto = luminancia(0xE2 as f64 / 255.0, 0xE2 as f64 / 255.0, 0xE6 as f64 / 255.0);
        let mut mostrados = 0;
        for it in self.itens.borrow().iter() {
            let (x, y, w, h) = retangulo(&it.c.borrow(), sw);
            let y = y - BARRA_ALTURA;
            let cabe = x >= 0 && x + w <= sw && y >= 0 && y + h <= sh;
            let mostrar = (cabe || self.editando.get()) && (!compacta || it.c.borrow().tipo == "relogio" || self.editando.get());
            it.anim.set_visible(mostrar);
            if !mostrar {
                continue;
            }
            mostrados += 1;
            self.fixo.move_(&it.anim, f64::from(x - FOLGA), f64::from(y - FOLGA));
            // a base when the wallpaper under it is too light for the text
            let base = luminancia_do_papel(mw, mh, x, it.c.borrow().y, w, h).is_some_and(|l| contraste(texto, l) < 4.5);
            let wd = it.widget.borrow();
            if base { wd.add_css_class("com-base") } else { wd.remove_css_class("com-base") }
        }
        if debug() {
            eprintln!("debug area: {} ({mw}x{mh}, {mostrados} widgets)", if compacta { "só o relógio" } else { "completa" });
        }
    }

    /// The desktop's menu (design "Menus de contexto", ÁREA DE TRABALHO): a new folder or document in the Desktop
    /// folder (opened where it is), paste the copied files there, the widgets, the wallpaper, the display, the agent.
    fn menu_da_area(self: &Rc<Self>, x: f64, y: f64) {
        use crate::context_menu::Menu;
        use kalango_i18n::tr;
        let m = Menu::at(&self.fixo, x, y);
        let c = m.main.clone();
        let s = self.shell.borrow().clone();
        let mesa = crate::pastas::especial(glib::UserDirectory::Desktop, "Área de Trabalho");
        {
            let (s, mesa) = (s.clone(), mesa.clone());
            let b = m.item(&c, "pasta", tr("New folder"), "Ctrl+Shift+N", "", move || {
                let p = nome_livre(&mesa, tr("New folder"), "");
                if std::fs::create_dir_all(&p).is_ok() {
                    if debug() {
                        eprintln!("debug area: nova pasta {}", p.display());
                    }
                    if let Some(s) = s.upgrade() {
                        s.abrir_arquivos(Some(&mesa));
                    }
                }
            });
            crate::central::posicao_debug("area menu nova pasta", b.upcast_ref());
        }
        let docs = m.sub(&c, "doc", tr("New document"), "novo-documento");
        for (nome, ext) in [(tr("Text document"), "txt"), (tr("Markdown note"), "md")] {
            let (s, mesa) = (s.clone(), mesa.clone());
            m.item(&docs, "doc", nome, "", "", move || {
                let p = nome_livre(&mesa, tr("New document"), ext);
                if std::fs::create_dir_all(&mesa).is_ok() && std::fs::write(&p, "").is_ok() {
                    if debug() {
                        eprintln!("debug area: novo documento {}", p.display());
                    }
                    if let Some(s) = s.upgrade() {
                        crate::editor::open(&s, Some(p));
                    }
                }
            });
        }
        {
            let mesa = mesa.clone();
            let colar = m.item(&c, "colar", tr("Paste"), "Ctrl+V", "", move || {
                let Some(d) = gdk::Display::default() else { return };
                let mesa = mesa.clone();
                d.clipboard().read_value_async(gdk::FileList::static_type(), glib::Priority::DEFAULT, gio::Cancellable::NONE, move |r| {
                    let Ok(v) = r else { return };
                    let Ok(l) = v.get::<gdk::FileList>() else { return };
                    let _ = std::fs::create_dir_all(&mesa);
                    for f in l.files() {
                        let Some(o) = f.path() else { continue };
                        let nome = o.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let (base, ext) = match nome.rsplit_once('.') {
                            Some((b, e)) if !b.is_empty() && o.is_file() => (b.to_string(), e.to_string()),
                            _ => (nome.clone(), String::new()),
                        };
                        let destino = nome_livre(&mesa, &base, &ext);
                        let feito = if o.is_dir() {
                            std::process::Command::new("cp").arg("-r").arg(&o).arg(&destino).status().is_ok_and(|s| s.success())
                        } else {
                            std::fs::copy(&o, &destino).is_ok()
                        };
                        if debug() {
                            eprintln!("debug area: colado {} ({})", destino.display(), if feito { "ok" } else { "falhou" });
                        }
                    }
                });
            });
            // (only when files were copied)
            if let Some(d) = gdk::Display::default() {
                colar.set_sensitive(d.clipboard().formats().contains_type(gdk::FileList::static_type()));
            }
        }
        Menu::divider(&c);
        {
            let w = Rc::downgrade(self);
            m.item(&c, "grade", tr("Edit widgets"), "", "", move || {
                if let Some(a) = w.upgrade() {
                    a.editar();
                }
            });
        }
        {
            let s = s.clone();
            m.item(&c, "imagem", tr("Change the wallpaper"), "", "", move || {
                if let Some(s) = s.upgrade() {
                    s.abrir_configuracoes(Some("aparencia/papel"));
                }
            });
        }
        {
            let s = s.clone();
            m.item(&c, "monitor", tr("Display settings"), "", "", move || {
                if let Some(s) = s.upgrade() {
                    s.abrir_configuracoes(Some("tela"));
                }
            });
        }
        Menu::divider(&c);
        {
            let s = s.clone();
            m.item(&c, "spark", tr("Ask the agent…"), tr("Super+Space"), "agente", move || {
                if let Some(s) = s.upgrade() {
                    s.comando().abrir("");
                }
            });
        }
        m.show();
        if debug() {
            eprintln!("debug area: menu aberto");
        }
    }

    /// Edit mode: above the windows, the widgets breathe and get their outline; the gallery comes from the right.
    pub fn editar(self: &Rc<Self>) {
        if self.editando.replace(true) {
            return;
        }
        // the gallery takes the right side: a floating side bar there closes (a pinned one has its own room)
        if let Some(s) = crate::shell_atual() {
            s.right_bar().close_if_floating();
        }
        self.janela.set_layer(Layer::Overlay);
        crate::superficies::take_keyboard(&self.janela);
        self.fundo.queue_draw();
        for it in self.itens.borrow().iter() {
            self.decorar(it);
        }
        self.posicionar();
        // the right column steps away from the gallery when the screen is wide enough for both columns and a free
        // middle beside it (on a smaller one, "Esconder galeria" uncovers what is under it)
        let (sw, _) = self.tamanho.get();
        self.set_gallery_room(if sw - GALLERY_ROOM >= 1200 { GALLERY_ROOM } else { 0 });
        if let Some(g) = self.galeria.borrow().clone() {
            g.abrir(true);
            if debug() {
                // (on screen: the surface starts under the top bar)
                let (b, j) = (g.toggle_button(), self.janela.clone());
                glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
                    if let Some(r) = b.compute_bounds(&j) {
                        eprintln!("debug central: galeria alternar em {} {}", (r.x() + r.width() / 2.0) as i32, BARRA_ALTURA + (r.y() + r.height() / 2.0) as i32);
                    }
                });
            }
        }
        // "breathing": scale 0.98 ↔ 1 over 1.6 s
        let w = Rc::downgrade(self);
        let t0 = std::time::Instant::now();
        self.fixo.add_tick_callback(move |_, _| {
            let Some(a) = w.upgrade().filter(|a| a.editando.get()) else { return glib::ControlFlow::Break };
            let fase = t0.elapsed().as_secs_f64() / 1.6 * std::f64::consts::TAU;
            for (i, it) in a.itens.borrow().iter().enumerate() {
                if !it.moldura.has_css_class("arrastando") {
                    it.anim.definir(Canal::Escala, 0.99 + 0.01 * (fase + i as f64 * 0.7).sin());
                }
            }
            glib::ControlFlow::Continue
        });
        if debug() {
            eprintln!("debug widgets: editando");
            self.registrar_itens();
        }
    }

    /// Done: the layout is saved and the desktop goes back under the windows.
    pub fn sair_da_edicao(self: &Rc<Self>) {
        if !self.editando.replace(false) {
            return;
        }
        self.guardar_layout();
        if let Err(e) = salvar(&self.layout.borrow()) {
            eprintln!("kalango-shell: widgets: {e}");
        }
        for it in self.itens.borrow().iter() {
            it.anim.ir(Canal::Escala, 1.0, mv::RAPIDA);
            self.decorar(it);
        }
        if let Some(g) = self.galeria.borrow().clone() {
            g.fechar();
        }
        self.set_gallery_room(0);
        self.janela.set_layer(Layer::Bottom);
        self.janela.set_keyboard_mode(KeyboardMode::OnDemand);
        self.fundo.queue_draw();
        self.posicionar();
        if debug() {
            eprintln!("debug widgets: concluído ({} widgets salvos)", self.layout.borrow().len());
        }
    }

    /// Drag (move), the handle (size) and "−" (remove) of one widget.
    fn ligar_item(self: &Rc<Self>, it: &Rc<Item>) {
        // remove
        let (w, i) = (Rc::downgrade(self), Rc::downgrade(it));
        it.remover.connect_clicked(move |_| {
            let (Some(a), Some(it)) = (w.upgrade(), i.upgrade()) else { return };
            a.itens.borrow_mut().retain(|x| !Rc::ptr_eq(x, &it));
            let anim = it.anim.clone();
            let f = a.fixo.clone();
            it.anim.sair((0.0, 0.0), 0.9, move || f.remove(&anim));
            a.guardar_layout();
            if debug() {
                eprintln!("debug widgets: {} removido", it.c.borrow().tipo);
            }
        });
        // move
        let arrasto = gtk::GestureDrag::new();
        let (w, i) = (Rc::downgrade(self), Rc::downgrade(it));
        arrasto.connect_drag_update(move |_, dx, dy| {
            let (Some(a), Some(it)) = (w.upgrade(), i.upgrade()) else { return };
            if !a.editando.get() {
                return;
            }
            it.moldura.add_css_class("arrastando");
            it.anim.definir(Canal::X, dx).definir(Canal::Y, dy).definir(Canal::Escala, 1.0);
            // the gallery steps aside: every cell is a place to drop it
            if let Some(g) = a.galeria.borrow().clone() {
                g.give_way(true);
            }
            let (sw, _) = a.tela();
            let (x, y, _, _) = retangulo(&it.c.borrow(), sw);
            let alvo = encaixar(&it.c.borrow().tipo, it.c.borrow().tam, x + dx as i32, y + dy as i32, sw);
            a.guias_em(&alvo);
        });
        let (w, i) = (Rc::downgrade(self), Rc::downgrade(it));
        arrasto.connect_drag_end(move |_, dx, dy| {
            let (Some(a), Some(it)) = (w.upgrade(), i.upgrade()) else { return };
            if !a.editando.get() {
                return;
            }
            it.moldura.remove_css_class("arrastando");
            a.esconder_guias();
            if let Some(g) = a.galeria.borrow().clone() {
                g.give_way(false);
            }
            let (sw, sh) = a.tela();
            let (x, y, _, _) = retangulo(&it.c.borrow(), sw);
            let alvo = encaixar(&it.c.borrow().tipo, it.c.borrow().tam, x + dx as i32, y + dy as i32, sw);
            let idx = a.itens.borrow().iter().position(|o| Rc::ptr_eq(o, &it));
            let layout: Vec<Colocado> = a.itens.borrow().iter().map(|o| o.c.borrow().clone()).collect();
            if !cabe(&alvo, &layout, idx, sw, sh) {
                // back where it was (mola/rápida)
                it.anim.ir(Canal::X, 0.0, mv::RAPIDA).ir(Canal::Y, 0.0, mv::RAPIDA);
                return;
            }
            // to the cell: from where it was dropped, with the quick spring
            let (nx, ny, _, _) = retangulo(&alvo, sw);
            // the same item, in its new place: its "−", its handle and this drag still work on it
            *it.c.borrow_mut() = alvo.clone();
            a.posicionar();
            it.anim.definir(Canal::X, f64::from(x + dx as i32 - nx)).definir(Canal::Y, f64::from(y + dy as i32 - ny));
            it.anim.ir(Canal::X, 0.0, mv::RAPIDA).ir(Canal::Y, 0.0, mv::RAPIDA);
            a.guardar_layout();
            if debug() {
                eprintln!("debug widgets: {} em {} {} ({})", alvo.tipo, alvo.x, alvo.y, if alvo.direita { "direita" } else { "esquerda" });
                a.registrar_itens();
            }
        });
        it.moldura.add_controller(arrasto);
        // size: the handle goes through the sizes the kind has (the nearest to where it is dragged)
        let tamanho = gtk::GestureDrag::new();
        tamanho.set_propagation_phase(gtk::PropagationPhase::Capture);
        let (w, i) = (Rc::downgrade(self), Rc::downgrade(it));
        tamanho.connect_drag_end(move |g, dx, dy| {
            let (Some(a), Some(it)) = (w.upgrade(), i.upgrade()) else { return };
            g.set_state(gtk::EventSequenceState::Claimed);
            let Some(t) = TIPOS.iter().find(|t| t.id == it.c.borrow().tipo) else { return };
            let (w0, h0) = it.c.borrow().tam.px();
            let (qw, qh) = (f64::from(w0) + dx, f64::from(h0) + dy);
            let melhor = t.tamanhos.iter().copied().min_by_key(|s| { let (w, h) = s.px(); ((f64::from(w) - qw).abs() + (f64::from(h) - qh).abs()) as i64 });
            if let Some(novo) = melhor.filter(|n| *n != it.c.borrow().tam) {
                a.mudar_tamanho(&it, novo);
            }
        });
        tamanho.connect_drag_begin(|g, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
        });
        it.alca.add_controller(tamanho);
    }

    fn mudar_tamanho(self: &Rc<Self>, it: &Rc<Item>, tam: Tam) {
        let (sw, sh) = self.tela();
        let mut c = it.c.borrow().clone();
        c.tam = tam;
        let idx = self.itens.borrow().iter().position(|o| Rc::ptr_eq(o, it));
        let layout: Vec<Colocado> = self.itens.borrow().iter().map(|o| o.c.borrow().clone()).collect();
        if !cabe(&c, &layout, idx, sw, sh) {
            if debug() {
                eprintln!("debug widgets: {} não cabe em {tam:?}", c.tipo);
            }
            return;
        }
        let Some(i) = idx else { return };
        self.fixo.remove(&it.anim);
        if let Some(novo) = self.item(c.clone()) {
            novo.anim.entrar((0.0, 0.0), 0.96);
            self.itens.borrow_mut()[i] = novo;
        }
        self.posicionar();
        self.guardar_layout();
        if debug() {
            eprintln!("debug widgets: {} agora {tam:?}", c.tipo);
            self.registrar_itens();
        }
    }

    /// From the gallery: in the first free cell.
    pub fn adicionar(self: &Rc<Self>, tipo: &str, tam: Tam) -> bool {
        let (sw, sh) = self.tela();
        let layout: Vec<Colocado> = self.itens.borrow().iter().map(|o| o.c.borrow().clone()).collect();
        let Some(c) = lugar_livre(tipo, tam, &layout, sw, sh) else { return false };
        let Some(it) = self.item(c.clone()) else { return false };
        it.anim.entrar((0.0, 12.0), 0.9);
        self.itens.borrow_mut().push(it);
        self.posicionar();
        self.guardar_layout();
        // (added from elsewhere — an answer pinned as a widget — with no "Concluir" to save it: saved now)
        if !self.editando.get() {
            if let Err(e) = salvar(&self.layout.borrow()) {
                eprintln!("kalango-shell: widgets: {e}");
            }
        }
        if debug() {
            eprintln!("debug widgets: {tipo} adicionado em {} {}", c.x, c.y);
            self.registrar_itens();
        }
        true
    }

    /// "Monte para mim": a new layout from the widgets the model chose.
    pub fn aplicar(self: &Rc<Self>, escolhidos: &[(String, Tam)]) {
        let (sw, sh) = self.tela();
        *self.layout.borrow_mut() = dispor(escolhidos, sw, sh);
        self.construir();
        for it in self.itens.borrow().iter() {
            self.decorar(it);
        }
        if debug() {
            eprintln!("debug widgets: disposição com {} widgets", self.itens.borrow().len());
        }
    }

    fn guias_em(&self, c: &Colocado) {
        let (sw, sh) = self.tamanho.get();
        let (x, y, w, h) = retangulo(c, sw);
        let y = y - BARRA_ALTURA;
        self.guia_v.set_size_request(1, sh);
        self.guia_h.set_size_request(sw, 1);
        self.guias.move_(&self.guia_v, f64::from(x), 0.0);
        self.guias.move_(&self.guia_h, 0.0, f64::from(y));
        let nome = match c.tam { Tam::P => "P", Tam::M => "M", Tam::G => "G" };
        self.medida.set_text(&format!("{w} × {h} · {nome}"));
        self.guias.move_(&self.medida, f64::from(x), f64::from(y + h + 8));
        for g in [self.guia_v.upcast_ref::<gtk::Widget>(), self.guia_h.upcast_ref(), self.medida.upcast_ref()] {
            g.set_visible(true);
        }
    }

    fn esconder_guias(&self) {
        for g in [self.guia_v.upcast_ref::<gtk::Widget>(), self.guia_h.upcast_ref(), self.medida.upcast_ref()] {
            g.set_visible(false);
        }
    }

    /// Debug: every widget's middle on screen (the session test drags them).
    fn registrar_itens(&self) {
        // (where they are laid out now: beside the gallery while editing)
        let (sw, _) = self.tela();
        for it in self.itens.borrow().iter() {
            let (x, y, w, h) = retangulo(&it.c.borrow(), sw);
            eprintln!("debug widgets: {} {:?} meio em {} {} alça em {} {} remover em {} {}", it.c.borrow().tipo, it.c.borrow().tam, x + w / 2, y + h / 2, x + w - 2, y + h - 2, x - 4, y - 4);
        }
    }

    /// Debug: where the ask field is on the screen (the session test types there).
    fn registrar_pergunta(&self) {
        let Some(p) = self.pergunta.borrow().clone() else { return };
        let campo = p.first_child().and_then(|l| l.next_sibling());
        match campo.as_ref().and_then(|c| c.compute_bounds(&self.janela)).filter(|_| p.is_mapped()) {
            Some(b) => eprintln!("debug area: pergunta em {} {}", (b.x() + b.width() / 3.0) as i32, (f64::from(BARRA_ALTURA) + f64::from(b.y() + b.height() / 2.0)) as i32),
            None => eprintln!("debug area: pergunta sem posição (mapeada={})", p.is_mapped()),
        }
    }
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_follows_wcag() {
        let branco = luminancia(1.0, 1.0, 1.0);
        let preto = luminancia(0.0, 0.0, 0.0);
        assert!((contraste(branco, preto) - 21.0).abs() < 0.01);
        // the widgets' text on the dark wallpaper reads well; on a light grey it does not
        let texto = luminancia(0xE2 as f64 / 255.0, 0xE2 as f64 / 255.0, 0xE6 as f64 / 255.0);
        assert!(contraste(texto, luminancia(0.086, 0.09, 0.106)) > 4.5);
        assert!(contraste(texto, luminancia(0.75, 0.75, 0.75)) < 4.5);
    }

    /// Dropping snaps to the 184 px grid, from the nearer edge; a full screen packs widgets without overlap.
    #[test]
    fn widgets_snap_and_pack() {
        let c = encaixar("cpu", Tam::P, 240, 250, 1920);
        assert_eq!((c.x, c.y, c.direita), (232, 248, false));
        let d = encaixar("gpu", Tam::P, 1700, 70, 1920);
        assert_eq!((d.x, d.y, d.direita), (48, 64, true));
        let v = dispor(&[("resumo".into(), Tam::G), ("gpu".into(), Tam::P), ("hoje".into(), Tam::M), ("cpu".into(), Tam::P)], 1920, 1080);
        assert_eq!(v.len(), 4);
        for (i, a) in v.iter().enumerate() {
            assert!(cabe(a, &v, Some(i), 1920, 1080), "{a:?}");
        }
        // a free cell is never on top of another widget
        let livre = lugar_livre("clima", Tam::P, &v, 1920, 1080).unwrap();
        assert!(cabe(&livre, &v, None, 1920, 1080));
    }

    #[test]
    fn a_layout_survives_saving() {
        let v = padrao();
        let s = serde_json::to_vec(&v).unwrap();
        assert_eq!(serde_json::from_slice::<Vec<Colocado>>(&s).unwrap(), v);
        assert_eq!(Tam::G.px(), (352, 352));
        assert_eq!(Tam::M.px().0 + 16, 2 * PASSO);
    }
}

/// A name not taken in `pasta`: "Nova pasta", "Nova pasta 2"… (`ext` without the dot; empty for a folder).
pub(crate) fn nome_livre(pasta: &std::path::Path, base: &str, ext: &str) -> std::path::PathBuf {
    let nome = |n: usize| {
        let b = if n == 1 { base.to_string() } else { format!("{base} {n}") };
        if ext.is_empty() { b } else { format!("{b}.{ext}") }
    };
    (1..).map(|n| pasta.join(nome(n))).find(|p| !p.exists()).unwrap()
}
