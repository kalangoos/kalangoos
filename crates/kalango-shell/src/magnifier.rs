//! The magnifier ("Lupa", design 60426, Super and +): the compositor draws it (KalangoOS's niri, patch 0007: a lens
//! around the pointer that follows it every frame, or the whole screen, or half of it); the shell keeps how much bigger
//! and shows, at the top of the screen, how much and the keys: Super = closer, Super − farther, Super 0 out
//! (`kalango-shell lupa mais|menos|sair|modo lente|tela|metade`, bound in niri's configuration; Super+Shift and the
//! wheel change it continuously).

use crate::conversa::ui_icon;
use crate::superficies::{caixa, camada, rotulo};
use gtk::prelude::*;
use gtk::{gio, Align, Orientation};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use kalango_i18n::tr;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The steps of Super = and Super −.
const STEPS: [f64; 9] = [1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0];

/// The next step up (or down) from `now`.
pub fn step(now: f64, up: bool) -> f64 {
    if up {
        STEPS.iter().copied().find(|s| *s > now + 0.01).unwrap_or(STEPS[STEPS.len() - 1])
    } else {
        STEPS.iter().rev().copied().find(|s| *s < now - 0.01).unwrap_or(1.0)
    }
}

/// "250%"
pub fn percent(f: f64) -> String {
    format!("{}%", (f * 100.0).round() as i64)
}

struct Hint {
    window: gtk::ApplicationWindow,
    pct: gtk::Label,
    how: gtk::Label,
}

thread_local! {
    static FACTOR: Cell<f64> = const { Cell::new(1.0) };
    static MODE: RefCell<String> = RefCell::new("lens".into());
    static HINT: RefCell<Option<Rc<Hint>>> = const { RefCell::new(None) };
}

fn key(text: &str) -> gtk::Label {
    rotulo(text, "lupa-tecla")
}

fn group(keys: &[&str], what: &str) -> gtk::Box {
    let g = caixa(Orientation::Horizontal, 4, "");
    for k in keys {
        g.append(&key(k));
    }
    let l = rotulo(what, "lupa-acao");
    l.set_margin_start(0);
    g.append(&l);
    g
}

fn hint() -> Option<Rc<Hint>> {
    if let Some(h) = HINT.with(|h| h.borrow().clone()) {
        return Some(h);
    }
    let app = gio::Application::default().and_downcast::<gtk::Application>()?;
    let window = camada(&app, "kalango-lupa", Layer::Overlay, &[Edge::Top]);
    window.set_exclusive_zone(-1);
    window.set_margin(Edge::Top, 44);
    window.add_css_class("lupa-janela");
    let card = caixa(Orientation::Horizontal, 14, "lupa-atalhos");
    card.set_size_request(520, -1);
    card.append(&group(&["Super", "="], tr("closer")));
    card.append(&group(&["Super", "−"], tr("farther")));
    card.append(&group(&["Super", "0"], tr("out")));
    let gap = caixa(Orientation::Horizontal, 0, "");
    gap.set_hexpand(true);
    card.append(&gap);
    // how much bigger, and how (the design's chip under the lens: the compositor draws the lens, the shell does not
    // know where the pointer is, so it is told here)
    let chip = caixa(Orientation::Horizontal, 8, "lupa-zoom");
    chip.set_valign(Align::Center);
    chip.append(&ui_icon("busca", "#E2E2E6", 16));
    let pct = rotulo("", "lupa-zoom-pct");
    chip.append(&pct);
    let how = rotulo("", "lupa-zoom-modo");
    chip.append(&how);
    card.append(&chip);
    window.set_child(Some(&card));
    let h = Rc::new(Hint { window, pct, how });
    HINT.with(|x| *x.borrow_mut() = Some(h.clone()));
    Some(h)
}

fn apply(f: f64) {
    let f = if f < 1.01 { 1.0 } else { f };
    FACTOR.with(|x| x.set(f));
    let mode = MODE.with(|m| m.borrow().clone());
    let r = crate::servicos::niri_acao(json!({"SetZoom": {"factor": f, "mode": mode}}));
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug lupa: {} ({mode}){}", percent(f), r.err().map(|e| format!(" — o niri recusou: {e}")).unwrap_or_default());
    }
    let Some(h) = hint() else { return };
    if f <= 1.0 {
        h.window.set_visible(false);
        return;
    }
    h.pct.set_text(&percent(f));
    h.how.set_text(match mode.as_str() {
        "full" => tr("whole screen"),
        "half" => tr("half of the screen"),
        _ => tr("follows the pointer"),
    });
    h.window.present();
}

/// `kalango-shell lupa …`
pub fn command(arg: Option<&str>, value: Option<&str>) {
    let now = FACTOR.with(|x| x.get());
    match arg {
        Some("mais") | None => apply(step(now, true)),
        Some("menos") => apply(step(now, false)),
        // the wheel: a little at a time
        Some("roda-mais") => apply((now * 1.1).clamp(1.25, 8.0)),
        Some("roda-menos") => apply(if now * 0.9 < 1.2 { 1.0 } else { now * 0.9 }),
        Some("sair") => apply(1.0),
        Some("modo") => {
            let m = match value {
                Some("tela") | Some("full") => "full",
                Some("metade") | Some("half") => "half",
                _ => "lens",
            };
            MODE.with(|x| *x.borrow_mut() = m.to_string());
            apply(if now > 1.0 { now } else { 2.0 });
        }
        Some(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closer_and_farther_go_by_steps() {
        assert_eq!(step(1.0, true), 1.5);
        assert_eq!(step(2.0, true), 2.5);
        assert_eq!(step(8.0, true), 8.0);
        assert_eq!(step(2.5, false), 2.0);
        assert_eq!(step(1.5, false), 1.0);
        // from a wheel's in-between value, to the next step
        assert_eq!(step(2.2, true), 2.5);
        assert_eq!(step(2.2, false), 2.0);
        assert_eq!(percent(2.5), "250%");
    }
}
