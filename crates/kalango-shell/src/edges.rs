//! The screen's edges and corners (Settings › Appearance › Dock and multitasking): resting the pointer against the
//! left edge opens Today and against the right one the notices' bar ("Abrir ao encostar o ponteiro na borda"), and
//! each corner can do something — the overview, the agent's bar, the notices, Today, the desktop — or nothing
//! ("Cantos ativos"). They are thin layer surfaces along the edges; the pointer must rest there a moment (a pass on the
//! way to a scroll bar does nothing).
//!
//! "Área de trabalho" (and Super+D): the desktop is shown by going to the empty space niri always keeps at the end of
//! the output; again, back to where the person was.

use crate::superficies::camada;
use gtk::prelude::*;
use gtk::glib;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::n_;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::time::Duration;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// What a corner can do (id, name).
pub const CORNER_ACTIONS: &[(&str, &str)] = &[
    ("nada", n_("Nothing")),
    ("visao", n_("Overview")),
    ("area", n_("Desktop")),
    ("agente", n_("Agent bar")),
    ("avisos", n_("Notices")),
    ("hoje", n_("Today")),
];

/// The four corners (id, default action), in the design's order: top left, top right, bottom left, bottom right.
pub const CORNERS: &[(&str, &str)] = &[("sup_esq", "visao"), ("sup_dir", "nada"), ("inf_esq", "area"), ("inf_dir", "agente")];

pub fn edges_open_bars() -> bool {
    crate::configuracoes::ajustes()["barras_na_borda"].as_bool().unwrap_or(true)
}

pub fn corner_action(corner: &str) -> String {
    let def = CORNERS.iter().find(|(c, _)| *c == corner).map(|(_, d)| *d).unwrap_or("nada");
    crate::configuracoes::ajustes()["cantos"][corner].as_str().unwrap_or(def).to_string()
}

pub fn set_corner_action(corner: &str, action: &str) {
    let mut c = crate::configuracoes::ajustes()["cantos"].clone();
    if !c.is_object() {
        c = json!({});
    }
    c[corner] = json!(action);
    crate::configuracoes::salvar_ajuste("cantos", c);
    if debug() {
        eprintln!("debug cantos: {corner} → {action}");
    }
    apply();
}

thread_local! {
    /// until when the edges rest: a window being dragged (and a moment after it is dropped, the pointer still there)
    static DRAG_UNTIL: Cell<Option<std::time::Instant>> = const { Cell::new(None) };
    static ZONES: RefCell<Vec<gtk::ApplicationWindow>> = const { RefCell::new(Vec::new()) };
    /// the edge that brings a hidden dock back (shown only while the dock is out)
    static DOCK_ZONE: RefCell<Option<gtk::ApplicationWindow>> = const { RefCell::new(None) };
    /// the space the person was in before "Área de trabalho"
    static BEFORE_DESKTOP: Cell<Option<u64>> = const { Cell::new(None) };
}

/// A window is being dragged (niri's `WindowDragChanged`): dropping it on an edge or in a corner to snap it must not
/// open a bar or the overview there.
pub fn window_drag() {
    DRAG_UNTIL.with(|d| d.set(Some(std::time::Instant::now() + Duration::from_millis(1500))));
}

/// Does what an edge or a corner is for.
fn act(action: &str) {
    if DRAG_UNTIL.with(|d| d.get()).is_some_and(|t| std::time::Instant::now() < t) {
        if debug() {
            eprintln!("debug bordas: {action} ignorado (arraste de janela)");
        }
        return;
    }
    let Some(s) = crate::shell_atual() else { return };
    if debug() {
        eprintln!("debug bordas: {action}");
    }
    match action {
        "visao" => s.visao().alternar(),
        "agente" => s.right_bar().abrir(crate::lateral::Modo::Agente),
        // the edge opens a closed bar (an open one stays as it is: its mode is not changed under the pointer)
        "direita" => {
            if !s.right_bar().aberta() {
                s.right_bar().abrir(crate::lateral::Modo::Avisos);
            }
        }
        "avisos" => s.right_bar().abrir(crate::lateral::Modo::Avisos),
        "hoje" | "esquerda" => {
            if !s.today_bar().aberta() {
                s.today_bar().alternar();
            }
        }
        "area" => show_desktop(),
        "dock" => crate::superficies::mostrar_dock(),
        _ => {}
    }
}

/// The desktop: to the empty space at the end of this output; again (on it), back to the space before.
pub fn show_desktop() {
    let Ok(v) = crate::servicos::niri_pedido(json!("Workspaces")) else { return };
    let espacos: Vec<Value> = v["Workspaces"].as_array().cloned().unwrap_or_default();
    let Some(atual) = espacos.iter().find(|w| w["is_focused"].as_bool() == Some(true)) else { return };
    let saida = atual["output"].clone();
    let atual_id = atual["id"].as_u64().unwrap_or(0);
    let vazio = atual["active_window_id"].is_null();
    if vazio {
        if let Some(antes) = BEFORE_DESKTOP.with(|b| b.take()) {
            let _ = crate::servicos::niri_acao(json!({"FocusWorkspace": {"reference": {"Id": antes}}}));
            if debug() {
                eprintln!("debug bordas: de volta ao espaço {antes}");
            }
        }
        return;
    }
    // the last space of this output is the empty one niri keeps
    let ultimo = espacos.iter().filter(|w| w["output"] == saida).max_by_key(|w| w["idx"].as_u64().unwrap_or(0));
    if let Some(u) = ultimo.and_then(|u| u["id"].as_u64()) {
        BEFORE_DESKTOP.with(|b| b.set(Some(atual_id)));
        let _ = crate::servicos::niri_acao(json!({"FocusWorkspace": {"reference": {"Id": u}}}));
        if debug() {
            eprintln!("debug bordas: área de trabalho (espaço {u})");
        }
    }
}

/// A thin surface on an edge or in a corner: the pointer resting on it `wait` does `action`.
fn zone(app: &gtk::Application, name: &str, anchors: &[Edge], size: (i32, i32), margins: &[(Edge, i32)], action: String, wait: u64) -> gtk::ApplicationWindow {
    // (over the bar and the windows: the corners are over the top bar's own ends)
    let w = camada(app, name, Layer::Overlay, anchors);
    // (no decoration; and a background that is not fully transparent — see style.css — or it takes no pointer)
    w.set_decorated(false);
    w.add_css_class("borda-ativa");
    w.set_keyboard_mode(KeyboardMode::None);
    w.set_exclusive_zone(-1);
    for (e, m) in margins {
        w.set_margin(*e, *m);
    }
    let area = gtk::DrawingArea::new();
    area.set_size_request(size.0, size.1);
    area.set_hexpand(true);
    area.set_vexpand(true);
    w.set_child(Some(&area));
    let motion = gtk::EventControllerMotion::new();
    let pending: std::rc::Rc<RefCell<Option<glib::SourceId>>> = Default::default();
    let p2 = pending.clone();
    let nome = name.to_string();
    motion.connect_enter(move |_, _, _| {
        if debug() {
            eprintln!("debug bordas: ponteiro em {nome}");
        }
        let a = action.clone();
        let p3 = p2.clone();
        *p2.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(wait), move || {
            p3.borrow_mut().take();
            act(&a);
        }));
    });
    motion.connect_leave(move |_| {
        if let Some(id) = pending.borrow_mut().take() {
            id.remove();
        }
    });
    w.add_controller(motion);
    w.present();
    w
}

/// The hidden dock's edge on or off (off while the dock shows: a fixed dock's last pixels take the clicks).
pub fn dock_zone(on: bool) {
    DOCK_ZONE.with(|z| {
        if let Some(w) = z.borrow().as_ref() {
            w.set_visible(on);
        }
    });
}

/// (Re)makes the zones as the settings say.
pub fn apply() {
    let Some(app) = gtk::gio::Application::default().and_downcast::<gtk::Application>() else { return };
    ZONES.with(|z| {
        for w in z.borrow_mut().drain(..) {
            w.close();
        }
    });
    let mut novas = Vec::new();
    let barra = crate::superficies::BARRA_ALTURA;
    // "Ocultar automaticamente": the dock's own edge brings it back (on a side dock, that is the left edge, Today's)
    let dock = crate::superficies::estilo_dock();
    let dock_na_esquerda = dock.ocultar && dock.estilo == "lateral";
    DOCK_ZONE.with(|z| z.borrow_mut().take());
    if dock.ocultar {
        let z = if dock_na_esquerda {
            zone(&app, "kalango-borda-dock", &[Edge::Top, Edge::Bottom, Edge::Left], (2, -1), &[(Edge::Top, barra + 40), (Edge::Bottom, 90)], "dock".into(), 120)
        } else {
            zone(&app, "kalango-borda-dock", &[Edge::Bottom, Edge::Left, Edge::Right], (-1, 2), &[(Edge::Left, 16), (Edge::Right, 16)], "dock".into(), 120)
        };
        z.set_visible(crate::superficies::dock_oculta());
        DOCK_ZONE.with(|x| *x.borrow_mut() = Some(z.clone()));
        novas.push(z);
    }
    if edges_open_bars() {
        // the sides between the top bar and the dock's height (the corners and the dock keep their own ends); 2 px:
        // the pointer stops on the last pixel, and a 1 px surface there may not take it
        for (nome, borda, acao) in [("kalango-borda-esquerda", Edge::Left, "esquerda"), ("kalango-borda-direita", Edge::Right, "direita")] {
            if borda == Edge::Left && dock_na_esquerda {
                continue;
            }
            novas.push(zone(&app, nome, &[Edge::Top, Edge::Bottom, borda], (2, -1), &[(Edge::Top, barra + 40), (Edge::Bottom, 90)], acao.to_string(), 300));
        }
    }
    for (canto, _) in CORNERS {
        let acao = corner_action(canto);
        if acao == "nada" {
            continue;
        }
        let ancoras: &[Edge] = match *canto {
            "sup_esq" => &[Edge::Top, Edge::Left],
            "sup_dir" => &[Edge::Top, Edge::Right],
            "inf_esq" => &[Edge::Bottom, Edge::Left],
            _ => &[Edge::Bottom, Edge::Right],
        };
        novas.push(zone(&app, &format!("kalango-canto-{canto}"), ancoras, (4, 4), &[], acao, 150));
    }
    if debug() {
        eprintln!("debug bordas: {} zonas (bordas {}{}, cantos {})", novas.len(), edges_open_bars(), if dock.ocultar { ", dock" } else { "" },
            CORNERS.iter().map(|(c, _)| format!("{c}={}", corner_action(c))).collect::<Vec<_>>().join(" "));
    }
    ZONES.with(|z| *z.borrow_mut() = novas);
}
