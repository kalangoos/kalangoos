//! Window layout picker (design "12 · Janelas, busca e sistema", frame "Layouts de janela (encaixe)").
//!
//! Super+Z (or `kalango-shell encaixe`): the picker drops from the top bar with the gentle spring. Five
//! layouts: Halves, Focus + side, Focus + stack, Three columns, Grid. Clicking one region of a layout snaps
//! the focused window there; clicking the layout's name applies the whole layout to the windows of the
//! workspace (the focused one first, then the most recently used). Esc or a click outside closes.
//!
//! The placing is done by niri (patched for KalangoOS): `SnapWindow { id, region: "x,y,w,h" }` in
//! thousandths of the working area. Dragging a window to an edge or corner snaps without the picker.
//!
//! While a window is dragged (niri event `WindowDragChanged`), bringing it near the top bar drops the
//! picker from the bar; its regions become drop targets (`SetSnapTargets`), the one under the pointer
//! lights up, and dropping there snaps the window into it while the other windows of the workspace take
//! the remaining regions of that layout, together.
//!
//! Under the layouts, the agent's suggestion: the arrangement used most with the apps open now
//! (`layout_habits`), with "Aplicar" — or the dragged window dropped on it.

use crate::servicos;
use crate::superficies::{caixa, camada, rotulo};
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{n_, tr, trf};
use kalango_movimento::{self as mv, Animavel, Canal};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A region in thousandths of the working area: x, y, w, h.
type Region = (u16, u16, u16, u16);

/// (name, regions in the order windows fill them)
const LAYOUTS: &[(&str, &[Region])] = &[
    (n_("Halves"), &[(0, 0, 500, 1000), (500, 0, 500, 1000)]),
    (n_("Focus + side"), &[(0, 0, 667, 1000), (667, 0, 333, 1000)]),
    (n_("Focus + stack"), &[(0, 0, 667, 1000), (667, 0, 333, 500), (667, 500, 333, 500)]),
    (n_("Three columns"), &[(0, 0, 333, 1000), (333, 0, 334, 1000), (667, 0, 333, 1000)]),
    (n_("Grid"), &[(0, 0, 500, 500), (500, 0, 500, 500), (0, 500, 500, 500), (500, 500, 500, 500)]),
];

const PREVIEW_W: f64 = 112.;
const PREVIEW_H: f64 = 70.;
const SLOT_GAP: f64 = 4.;

pub struct Encaixe {
    janela: gtk::ApplicationWindow,
    fundo: Animavel,
    cartao: Animavel,
    titulo: gtk::Label,
    aberta: Cell<bool>,
    /// every region button: (button, layout index, region index)
    areas: RefCell<Vec<(gtk::Button, usize, usize)>>,
    /// opened by a window drag (no keyboard, no dimming; closes when the drag ends)
    drag_mode: Cell<bool>,
    /// the dragged window
    dragged: Cell<Option<u64>>,
    /// region under the pointer during the drag
    alvo: Cell<Option<(usize, usize)>>,
    /// the agent's suggestion (design "Sugestão do agente") and the hint shown when there is none
    sugestao: gtk::Box,
    sug_titulo: gtk::Label,
    sug_texto: gtk::Label,
    dica: gtk::Label,
    atual: RefCell<Option<crate::layout_habits::Suggestion>>,
    /// the dragged window is over the suggestion
    sug_alvo: Cell<bool>,
    /// where the suggestion puts the dragged window (when its app is in it): the suggestion is a drop target then
    sug_regiao: Cell<Option<Region>>,
}

/// The pointer near the top bar opens the picker during a drag (the corners stay for quarters).
const DRAG_TOP: f64 = 52.;

fn region_arg(r: Region) -> String {
    format!("{},{},{},{}", r.0, r.1, r.2, r.3)
}

/// Windows of the focused workspace: the focused one first, then by most recent focus.
pub(crate) fn workspace_windows() -> Vec<Value> {
    let Ok(Value::Object(o)) = servicos::niri_pedido(json!("Windows")) else { return vec![] };
    let mut all = o.get("Windows").and_then(Value::as_array).cloned().unwrap_or_default();
    // (the focused window's space; while the picker holds the keyboard no window is focused: the focused space)
    let ws = all.iter().find(|w| w["is_focused"] == true).and_then(|w| w["workspace_id"].as_u64()).or_else(|| {
        let v = servicos::niri_pedido(json!("Workspaces")).ok()?;
        v["Workspaces"].as_array()?.iter().find(|w| w["is_focused"] == true)?["id"].as_u64()
    });
    all.retain(|w| ws.is_some() && w["workspace_id"].as_u64() == ws);
    let stamp = |w: &Value| {
        let t = &w["focus_timestamp"];
        (t["secs"].as_u64().unwrap_or(0), t["nanos"].as_u64().unwrap_or(0))
    };
    all.sort_by(|a, b| (b["is_focused"] == true).cmp(&(a["is_focused"] == true)).then(stamp(b).cmp(&stamp(a))));
    all
}

fn snap(id: Option<u64>, r: Region) {
    let _ = servicos::niri_acao(json!({"SnapWindow": {"id": id, "region": region_arg(r)}}));
}

impl Encaixe {
    pub fn new(app: &gtk::Application) -> Rc<Encaixe> {
        let janela = camada(app, "kalango-encaixe", Layer::Overlay, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
        janela.set_exclusive_zone(-1);
        let raiz = gtk::Overlay::new();
        let escuro = caixa(Orientation::Vertical, 0, "fundo-leve");
        escuro.set_hexpand(true);
        escuro.set_vexpand(true);
        let fundo = Animavel::new(&escuro);
        raiz.set_child(Some(&fundo));

        let painel = caixa(Orientation::Vertical, 12, "encaixe");
        painel.set_size_request(760, -1);
        let topo = caixa(Orientation::Horizontal, 8, "");
        topo.append(&crate::conversa::ui_icon("layout", "#A5A1E2", 16));
        let titulo = rotulo("", "encaixe-titulo");
        titulo.set_hexpand(true);
        titulo.set_xalign(0.0);
        titulo.set_ellipsize(gtk::pango::EllipsizeMode::End);
        titulo.set_width_chars(1);
        topo.append(&titulo);
        for (k, d) in [("←", tr("half")), ("↑", tr("maximize"))] {
            topo.append(&rotulo("Super", "tecla"));
            topo.append(&rotulo(k, "tecla"));
            topo.append(&rotulo(d, "fraco"));
        }
        painel.append(&topo);
        let linha = caixa(Orientation::Horizontal, 10, "");
        linha.set_homogeneous(true);
        painel.append(&linha);
        let sugestao = caixa(Orientation::Horizontal, 12, "encaixe-sugestao");
        sugestao.append(&crate::conversa::ui_icon("spark", "#A5A1E2", 16));
        let textos = caixa(Orientation::Vertical, 2, "");
        textos.set_hexpand(true);
        textos.set_valign(Align::Center);
        let sug_titulo = rotulo("", "encaixe-sug-titulo");
        let sug_texto = rotulo("", "encaixe-sug-texto");
        for l in [&sug_titulo, &sug_texto] {
            l.set_xalign(0.0);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.set_width_chars(1);
            textos.append(l);
        }
        sugestao.append(&textos);
        let aplicar = gtk::Button::with_label(tr("Apply"));
        aplicar.add_css_class("encaixe-aplicar");
        aplicar.set_valign(Align::Center);
        sugestao.append(&aplicar);
        sugestao.set_visible(false);
        painel.append(&sugestao);
        let dica = rotulo(tr("Click a region to snap the window there, or the name to arrange the workspace windows. Dragging a window to an edge or corner also snaps it."), "fraco");
        dica.set_wrap(true);
        dica.set_xalign(0.0);
        // the hint wraps inside the panel's width (760, as in the design) instead of widening it
        dica.set_max_width_chars(80);
        dica.set_width_chars(1);
        painel.append(&dica);

        let cartao = Animavel::new(&painel);
        cartao.origem(0.5, 0.0);
        cartao.set_halign(Align::Center);
        cartao.set_valign(Align::Start);
        cartao.set_margin_top(44);
        raiz.add_overlay(&cartao);
        janela.set_child(Some(&raiz));

        let e = Rc::new(Encaixe {
            janela, fundo, cartao, titulo, aberta: Cell::new(false), areas: RefCell::default(),
            drag_mode: Cell::new(false), dragged: Cell::new(None), alvo: Cell::new(None),
            sugestao, sug_titulo, sug_texto, dica, atual: RefCell::default(), sug_alvo: Cell::new(false),
            sug_regiao: Cell::new(None),
        });
        let w = Rc::downgrade(&e);
        aplicar.connect_clicked(move |_| {
            if let Some(e) = w.upgrade() {
                e.fechar();
                e.aplicar_sugestao();
            }
        });
        for (li, (name, regions)) in LAYOUTS.iter().enumerate() {
            linha.append(&e.layout_card(li, tr(name), regions));
        }
        let teclas = gtk::EventControllerKey::new();
        let w = Rc::downgrade(&e);
        teclas.connect_key_pressed(move |_, k, _, _| {
            if k == gdk::Key::Escape {
                if let Some(e) = w.upgrade() {
                    e.fechar();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        e.janela.add_controller(teclas);
        let clique = gtk::GestureClick::new();
        let w = Rc::downgrade(&e);
        clique.connect_pressed(move |_, _, x, y| {
            let Some(e) = w.upgrade() else { return };
            let dentro = e.cartao.compute_bounds(&e.janela).map(|b| b.contains_point(&gtk::graphene::Point::new(x as f32, y as f32))).unwrap_or(true);
            if !dentro {
                e.fechar();
            }
        });
        e.fundo.add_controller(clique);
        e
    }

    /// One layout: its regions (each clickable) and its name (applies the whole layout).
    fn layout_card(self: &Rc<Self>, li: usize, name: &str, regions: &'static [Region]) -> gtk::Box {
        let card = caixa(Orientation::Vertical, 8, "encaixe-layout");
        let preview = gtk::Fixed::new();
        preview.set_size_request(PREVIEW_W as i32, PREVIEW_H as i32);
        preview.set_halign(Align::Center);
        preview.add_css_class("encaixe-previa");
        for (ri, r) in regions.iter().enumerate() {
            let x = PREVIEW_W * f64::from(r.0) / 1000.;
            let y = PREVIEW_H * f64::from(r.1) / 1000.;
            let w = PREVIEW_W * f64::from(r.2) / 1000.;
            let h = PREVIEW_H * f64::from(r.3) / 1000.;
            let slot = gtk::Button::new();
            slot.add_css_class("encaixe-area");
            slot.set_size_request((w - SLOT_GAP).max(4.) as i32, (h - SLOT_GAP).max(4.) as i32);
            slot.set_tooltip_text(Some(tr("Snap the window here")));
            let me = Rc::downgrade(self);
            let r = *r;
            slot.connect_clicked(move |_| {
                if let Some(e) = me.upgrade() {
                    e.fechar();
                    snap(None, r);
                }
            });
            self.areas.borrow_mut().push((slot.clone(), li, ri));
            preview.put(&slot, x + SLOT_GAP / 2., y + SLOT_GAP / 2.);
        }
        card.append(&preview);
        let nome = gtk::Button::with_label(name);
        nome.add_css_class("encaixe-nome");
        nome.set_tooltip_text(Some(&trf("Arrange up to {n} windows like this", &[("n", regions.len().to_string())])));
        let me = Rc::downgrade(self);
        nome.connect_clicked(move |_| {
            if let Some(e) = me.upgrade() {
                e.fechar();
                let mut slots = Vec::new();
                for (i, (w, r)) in workspace_windows().iter().zip(regions.iter()).enumerate() {
                    snap(w["id"].as_u64(), *r);
                    slots.push((i, w["app_id"].as_str().unwrap_or("").to_string()));
                }
                crate::layout_habits::record(li, slots);
            }
        });
        card.append(&nome);
        card
    }

    /// A window drag moved (`id`) or ended (`None`); `x`, `y` in output coordinates.
    pub fn drag(self: &Rc<Self>, id: Option<u64>, x: f64, y: f64) {
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug arraste: {id:?} {x:.0} {y:.0} aberta={} arraste={} largura={}", self.aberta.get(), self.drag_mode.get(), self.janela.width());
        }
        match id {
            Some(id) => {
                self.dragged.set(Some(id));
                let w = self.output_width();
                if !self.aberta.get() {
                    // near the top bar, away from the corners (they are for quarters)
                    if y <= DRAG_TOP && x > w * 0.2 && x < w * 0.8 {
                        self.open_for_drag();
                    }
                    return;
                }
                if !self.drag_mode.get() {
                    return;
                }
                // moved well below the picker: it goes back up
                let bottom = self.cartao.compute_bounds(&self.janela).map(|b| f64::from(b.y() + b.height())).unwrap_or(300.);
                if y > bottom + 80. {
                    self.fechar();
                    return;
                }
                self.highlight(x, y);
            }
            None => {
                let dragged = self.dragged.take();
                if !(self.aberta.get() && self.drag_mode.get()) {
                    return;
                }
                let target = self.alvo.take();
                let on_suggestion = self.sug_alvo.replace(false);
                self.fechar();
                // dropped on the suggestion: niri put the window in its region; the others follow it
                if on_suggestion {
                    self.aplicar_sugestao();
                    return;
                }
                // dropped on a region: niri snapped the window; the others take the remaining regions
                if let (Some((li, ri)), Some(dragged)) = (target, dragged) {
                    let regions = LAYOUTS[li].1;
                    let todas = workspace_windows();
                    let app_of = |w: &Value| w["app_id"].as_str().unwrap_or("").to_string();
                    let mut slots: Vec<(usize, String)> = todas.iter().find(|w| w["id"].as_u64() == Some(dragged)).map(|w| vec![(ri, app_of(w))]).unwrap_or_default();
                    let others: Vec<&Value> = todas.iter().filter(|w| w["id"].as_u64() != Some(dragged)).collect();
                    let free = regions.iter().enumerate().filter(|(i, _)| *i != ri);
                    for (w, (i, r)) in others.iter().zip(free) {
                        snap(w["id"].as_u64(), *r);
                        slots.push((i, app_of(w)));
                    }
                    crate::layout_habits::record(li, slots);
                }
            }
        }
    }

    /// Width of the output: the surface's when it is mapped, else the monitor's (hidden = 0).
    fn output_width(&self) -> f64 {
        let w = self.janela.width();
        if w > 0 {
            return f64::from(w);
        }
        gdk::Display::default()
            .and_then(|d| d.monitors().item(0))
            .and_downcast::<gdk::Monitor>()
            .map(|m| f64::from(m.geometry().width()))
            .unwrap_or(1920.)
    }

    fn highlight(&self, x: f64, y: f64) {
        let p = gtk::graphene::Point::new(x as f32, y as f32);
        let sobre = self.sugestao.is_visible() && self.sug_regiao.get().is_some()
            && self.sugestao.compute_bounds(&self.janela).is_some_and(|r| r.contains_point(&p));
        self.sug_alvo.set(sobre);
        if sobre { self.sugestao.add_css_class("alvo") } else { self.sugestao.remove_css_class("alvo") }
        let mut found = None;
        for (b, li, ri) in self.areas.borrow().iter() {
            let over = b.compute_bounds(&self.janela).is_some_and(|r| r.contains_point(&p));
            if over {
                b.add_css_class("alvo");
                found = Some((*li, *ri));
            } else {
                b.remove_css_class("alvo");
            }
        }
        self.alvo.set(found);
    }

    /// Opened by a drag: drops from the bar without taking the keyboard; once laid out, its regions
    /// become niri's drop targets.
    fn open_for_drag(self: &Rc<Self>) {
        self.drag_mode.set(true);
        self.preparar_sugestao();
        self.sug_regiao.set(self.dragged_in_suggestion());
        self.titulo.set_text(tr("Drop on a region to arrange the windows"));
        self.aberta.set(true);
        self.fundo.definir(Canal::Opacidade, 0.0);
        self.janela.present();
        self.janela.set_keyboard_mode(KeyboardMode::None);
        self.cartao.entrar((0.0, -8.0), 0.96);
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(std::time::Duration::from_millis(60), move || {
            let Some(e) = w.upgrade() else { return };
            if !(e.aberta.get() && e.drag_mode.get()) {
                return;
            }
            // targets in output coordinates (the surface covers the whole output); the scale-in
            // animation is still running, so use where the regions will rest: the final layout
            let mut targets: Vec<Value> = e.areas.borrow().iter().filter_map(|(b, li, ri)| {
                let r = b.compute_bounds(&e.janela)?;
                let reg = LAYOUTS[*li].1[*ri];
                Some(json!({"x": r.x(), "y": r.y(), "w": r.width(), "h": r.height(), "region": region_arg(reg)}))
            }).collect();
            // the suggestion takes the dragged window into its place in it (when its app is in it)
            if let (Some(reg), Some(r)) = (e.sug_regiao.get(), e.sugestao.compute_bounds(&e.janela).filter(|_| e.sugestao.is_visible())) {
                targets.push(json!({"x": r.x(), "y": r.y(), "w": r.width(), "h": r.height(), "region": region_arg(reg)}));
            }
            let _ = servicos::niri_acao(json!({"SetSnapTargets": {"targets": Value::Array(targets).to_string()}}));
        });
    }

    pub fn abrir(&self) {
        self.drag_mode.set(false);
        self.preparar_sugestao();
        let focused = servicos::niri_pedido(json!("FocusedWindow")).ok().and_then(|v| v.get("FocusedWindow").cloned()).unwrap_or(Value::Null);
        let title = focused["title"].as_str().filter(|t| !t.is_empty()).or(focused["app_id"].as_str()).unwrap_or("").to_string();
        self.titulo.set_text(&if title.is_empty() { tr("Snap the window").to_string() } else { trf("Snap “{title}”", &[("title", title)]) });
        self.aberta.set(true);
        self.janela.present();
        let w = self.janela.downgrade();
        glib::timeout_add_local_once(std::time::Duration::from_millis(40), move || {
            if let Some(j) = w.upgrade() {
                crate::superficies::take_keyboard(&j);
            }
        });
        self.fundo.ir(Canal::Opacidade, 1.0, mv::SUAVE);
        self.cartao.entrar((0.0, -8.0), 0.96);
        // for the session test: where the first region (left half of "Halves") is, once laid out
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            let j = self.janela.downgrade();
            glib::timeout_add_local_once(std::time::Duration::from_millis(700), move || {
                let Some(j) = j.upgrade() else { return };
                // (and the suggestion's "Aplicar", when it shows)
                for (classe, nome) in [("encaixe-area", "área"), ("encaixe-aplicar", "aplicar")] {
                    let mut stack = vec![j.clone().upcast::<gtk::Widget>()];
                    while let Some(w) = stack.pop() {
                        if w.has_css_class(classe) {
                            if let Some(b) = w.compute_bounds(&j).filter(|_| w.is_mapped()) {
                                eprintln!("debug encaixe {nome}: {} {}", (b.x() + b.width() / 2.) as i32, (b.y() + b.height() / 2.) as i32);
                            }
                            break;
                        }
                        let mut c = w.last_child();
                        while let Some(x) = c {
                            stack.push(x.clone());
                            c = x.prev_sibling();
                        }
                    }
                }
            });
        }
    }

    pub fn fechar(&self) {
        if !self.aberta.replace(false) {
            return;
        }
        if self.drag_mode.replace(false) {
            // the drop targets go away with the picker
            let _ = servicos::niri_acao(json!({"SetSnapTargets": {"targets": "[]"}}));
            for (b, _, _) in self.areas.borrow().iter() {
                b.remove_css_class("alvo");
            }
        }
        self.janela.set_keyboard_mode(KeyboardMode::None);
        self.fundo.ir(Canal::Opacidade, 0.0, mv::RAPIDA);
        let j = self.janela.downgrade();
        let c = self.cartao.downgrade();
        self.cartao.sair((0.0, -8.0), 0.98, move || {
            if let (Some(j), Some(c)) = (j.upgrade(), c.upgrade()) {
                if c.valor(Canal::Opacidade) < 0.05 {
                    j.set_visible(false);
                }
            }
        });
    }

    pub fn alternar(&self) {
        if self.aberta.get() { self.fechar() } else { self.abrir() }
    }

    /// The suggestion for the windows of this space (or the hint, when there is none).
    fn preparar_sugestao(&self) {
        let abertos: Vec<String> = workspace_windows().iter().filter_map(|w| w["app_id"].as_str().map(String::from)).collect();
        let s = crate::layout_habits::suggestion_now(&abertos).filter(|s| LAYOUTS.get(s.layout).is_some());
        self.sugestao.remove_css_class("alvo");
        let Some(s) = s else {
            self.sugestao.set_visible(false);
            self.dica.set_visible(true);
            *self.atual.borrow_mut() = None;
            return;
        };
        let shell = crate::shell_atual();
        let app = |id: &str| shell.as_ref().and_then(|sh| crate::apps::da_janela(id, &sh.todos).cloned());
        let (nome, regions) = LAYOUTS[s.layout];
        let categorias: Vec<&str> = s.slots.iter().filter_map(|(_, id)| app(id)).map(|a| crate::apps::categoria(&a)).collect();
        let titulo = crate::layout_habits::title(&categorias, tr(nome));
        let texto = format!("{} ({})", crate::layout_habits::sentence(regions, &s.slots, |id| app(id).map(|a| a.nome).unwrap_or_else(|| id.to_string())),
            crate::layout_habits::reason(&s));
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug layouts: sugestão “{titulo}” — {texto}");
        }
        self.sug_titulo.set_text(&titulo);
        self.sug_texto.set_text(&texto);
        self.sugestao.set_visible(true);
        self.dica.set_visible(false);
        *self.atual.borrow_mut() = Some(s);
    }

    /// The suggested region of the dragged window's app, when the suggestion has it.
    fn dragged_in_suggestion(&self) -> Option<Region> {
        let id = self.dragged.get()?;
        let app = workspace_windows().into_iter().find(|w| w["id"].as_u64() == Some(id))?["app_id"].as_str()?.to_string();
        let s = self.atual.borrow().clone()?;
        let ri = s.slots.iter().find(|(_, a)| *a == app)?.0;
        LAYOUTS.get(s.layout)?.1.get(ri).copied()
    }

    /// The suggested arrangement: each app's window (the one used last) to its region; kept as used once more.
    fn aplicar_sugestao(&self) {
        let Some(s) = self.atual.borrow().clone() else { return };
        let Some((_, regions)) = LAYOUTS.get(s.layout) else { return };
        let mut janelas = workspace_windows();
        for (ri, app) in &s.slots {
            if let (Some(pos), Some(r)) = (janelas.iter().position(|w| w["app_id"].as_str() == Some(app.as_str())), regions.get(*ri)) {
                let w = janelas.remove(pos);
                snap(w["id"].as_u64(), *r);
            }
        }
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug layouts: sugestão aplicada (layout {})", s.layout);
        }
        crate::layout_habits::record(s.layout, s.slots.clone());
    }
}
