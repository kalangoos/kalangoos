//! The taskbar's window previews (design "Dock fixa · prévia de janela"): the pointer resting on an app with windows
//! shows, above it, a card for each of its windows (up to four), with the app's icon, the title, × to close it and
//! the window's own picture (niri's thumbnail). A click on a card goes to that window. Only in the taskbar style.

use crate::alternador::{carregar_miniatura, thumbs_dir};
use crate::apps;
use crate::conversa::ui_icon;
use crate::servicos::niri_acao;
use crate::superficies::{caixa, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

thread_local! {
    /// the preview showing (its popover, and the button whose tooltip it stands for) and the timer about to show or
    /// hide one
    static SHOWING: RefCell<Option<(gtk::Popover, glib::WeakRef<gtk::Button>)>> = const { RefCell::new(None) };
    static TIMER: RefCell<Option<glib::SourceId>> = const { RefCell::new(None) };
    /// the pointer is on the preview itself (it stays)
    static OVER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn cancel() {
    TIMER.with(|t| {
        if let Some(id) = t.borrow_mut().take() {
            id.remove();
        }
    });
}

fn later(ms: u64, f: impl FnOnce() + 'static) {
    cancel();
    let id = glib::timeout_add_local_once(Duration::from_millis(ms), move || {
        TIMER.with(|t| t.borrow_mut().take());
        f();
    });
    TIMER.with(|t| *t.borrow_mut() = Some(id));
}

pub fn hide() {
    if let Some((p, b)) = SHOWING.with(|s| s.borrow_mut().take()) {
        p.popdown();
        if let Some(b) = b.upgrade() {
            b.set_has_tooltip(true);
        }
    }
}

/// Follows the pointer on an app's button of the bar.
pub fn attach(button: &gtk::Button, shell: &Rc<Shell>, app_id: &str) {
    let motion = gtk::EventControllerMotion::new();
    let (b, s, id) = (button.downgrade(), Rc::downgrade(shell), app_id.to_string());
    motion.connect_enter(move |_, _, _| {
        if !crate::superficies::modo_barra_de_tarefas() {
            return;
        }
        let (b, s, id) = (b.clone(), s.clone(), id.clone());
        // a moment of rest first (a pass over the bar shows nothing); a preview already up follows at once
        let ms = if SHOWING.with(|x| x.borrow().is_some()) { 60 } else { 450 };
        later(ms, move || {
            if let (Some(b), Some(s)) = (b.upgrade(), s.upgrade()) {
                show(&b, &s, &id);
            }
        });
    });
    motion.connect_leave(|_| {
        later(300, || {
            if !OVER.with(|o| o.get()) {
                hide();
            }
        });
    });
    button.add_controller(motion);
}

fn show(button: &gtk::Button, shell: &Rc<Shell>, app_id: &str) {
    let mut windows: Vec<(u64, String, u64)> = shell.estado.janelas.borrow().values()
        .filter(|j| apps::da_janela(&j.app_id, &shell.todos).is_some_and(|a| a.id == app_id))
        .map(|j| (j.id, j.titulo.clone(), j.foco_em)).collect();
    if windows.is_empty() {
        hide();
        return;
    }
    windows.sort_by_key(|w| std::cmp::Reverse(w.2));
    windows.truncate(4);
    hide();
    let app = shell.todos.iter().find(|a| a.id == app_id).cloned();
    let pop = gtk::Popover::new();
    pop.add_css_class("previa-janela-pop");
    pop.set_has_arrow(false);
    pop.set_position(gtk::PositionType::Top);
    pop.set_autohide(false);
    let row = caixa(Orientation::Horizontal, 8, "");
    for (id, title, _) in &windows {
        let card = gtk::Button::new();
        card.add_css_class("previa-janela");
        let col = caixa(Orientation::Vertical, 8, "");
        let head = caixa(Orientation::Horizontal, 8, "");
        if let Some(a) = &app {
            head.append(&apps::imagem(a, 18));
        }
        let t = rotulo(title, "previa-janela-titulo");
        t.set_hexpand(true);
        t.set_xalign(0.0);
        t.set_ellipsize(gtk::pango::EllipsizeMode::End);
        t.set_width_chars(1);
        head.append(&t);
        let close = gtk::Button::new();
        close.add_css_class("previa-janela-fechar");
        close.set_child(Some(&ui_icon("fechar", "#8E8F99", 12)));
        close.set_tooltip_text(Some(kalango_i18n::tr("Close window")));
        let wid = *id;
        close.connect_clicked(move |_| {
            let _ = niri_acao(json!({"CloseWindow": {"id": wid}}));
            hide();
        });
        head.append(&close);
        col.append(&head);
        let pic = caixa(Orientation::Vertical, 0, "previa-janela-miniatura");
        pic.set_size_request(240, 135);
        col.append(&pic);
        card.set_child(Some(&col));
        let wid = *id;
        card.connect_clicked(move |_| {
            let _ = niri_acao(json!({"FocusWindow": {"id": wid}}));
            hide();
        });
        row.append(&card);
        request_picture(wid, &pic);
    }
    pop.set_child(Some(&row));
    let over = gtk::EventControllerMotion::new();
    over.connect_enter(|_, _, _| {
        OVER.with(|o| o.set(true));
        cancel();
    });
    over.connect_leave(|_| {
        OVER.with(|o| o.set(false));
        later(300, hide);
    });
    pop.add_controller(over);
    pop.set_parent(button);
    pop.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    pop.popup();
    // (the preview says the app's name already: no tooltip over it)
    button.set_has_tooltip(false);
    SHOWING.with(|s| *s.borrow_mut() = Some((pop, button.downgrade())));
    if debug() {
        eprintln!("debug prévia: {app_id} ({} janelas)", windows.len());
    }
}

/// The window's picture (niri's thumbnail) into the card, when it comes.
fn request_picture(id: u64, into: &gtk::Box) {
    let path = thumbs_dir().join(format!("previa-{id}.png"));
    let _ = std::fs::remove_file(&path);
    if niri_acao(json!({"ThumbnailWindow": {"id": id, "path": path.display().to_string(), "max_size": 480}})).is_err() {
        return;
    }
    let into = into.downgrade();
    let t0 = Instant::now();
    glib::timeout_add_local(Duration::from_millis(40), move || {
        let Some(b) = into.upgrade() else { return glib::ControlFlow::Break };
        if path.is_file() {
            let Some(pb) = carregar_miniatura(&path) else { return glib::ControlFlow::Continue };
            let _ = std::fs::remove_file(&path);
            // the window's proportion inside 240 × 135
            let k = (240.0 / f64::from(pb.width())).min(135.0 / f64::from(pb.height()));
            let (w, h) = ((f64::from(pb.width()) * k) as i32, (f64::from(pb.height()) * k) as i32);
            let p = crate::visao::figura(&pb, w, h);
            p.set_halign(Align::Center);
            p.set_valign(Align::Center);
            p.set_vexpand(true);
            b.append(&p);
            return glib::ControlFlow::Break;
        }
        if t0.elapsed() > Duration::from_secs(2) {
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}
