//! The taskbar's start menu (design "Dock fixa · Menu Iniciar aberto"): above the bar, in the middle of the screen.
//! - **Search:** filters the apps as the person types; Enter opens the first one, or, with no app, asks the agent.
//! - **Fixados:** the pinned apps, and "Todos os apps" (the grid).
//! - **Recomendado pelo agente:** recent files, the requests waiting for the person, and the next event of the day.
//! - **Footer:** the person, the agents and today's tokens, with Settings, Lock and Power.
//!
//! Esc, a click outside or the start button again closes it.

use crate::apps::{self, App};
use crate::conversa::ui_icon;
use crate::superficies::{caixa, camada, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{gdk, gio, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use kalango_movimento::{self as mv, Animavel, Canal};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

struct Menu {
    window: gtk::ApplicationWindow,
    card: Animavel,
    entry: gtk::Entry,
    grid: gtk::FlowBox,
    recommended: gtk::Grid,
    rec_title: gtk::Box,
    status: gtk::Label,
    open: Cell<bool>,
    shell: RefCell<std::rc::Weak<Shell>>,
}

thread_local! {
    static MENU: RefCell<Option<Rc<Menu>>> = const { RefCell::new(None) };
}

/// Opens or closes the start menu.
pub fn toggle(shell: &Rc<Shell>) {
    let m = MENU.with(|m| m.borrow().clone()).unwrap_or_else(|| {
        let m = Menu::new(shell);
        MENU.with(|x| *x.borrow_mut() = Some(m.clone()));
        m
    });
    if m.open.get() { m.close() } else { m.show() }
}

/// Closes it (another surface opened).
pub fn close() {
    if let Some(m) = MENU.with(|m| m.borrow().clone()) {
        m.close();
    }
}

fn icon_button(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("tb-botao");
    b.set_child(Some(&ui_icon(icon, "#8E8F99", 16)));
    b.set_tooltip_text(Some(tip));
    b.set_valign(Align::Center);
    b
}

impl Menu {
    fn new(shell: &Rc<Shell>) -> Rc<Menu> {
        let app = gtk::gio::Application::default().and_downcast::<gtk::Application>().expect("app");
        let window = camada(&app, "kalango-menu-iniciar", Layer::Overlay, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
        window.set_exclusive_zone(-1);
        let root = gtk::Overlay::new();
        // (the whole output, transparent but for a hint of shade, so a click outside closes)
        let back = caixa(Orientation::Vertical, 0, "menu-iniciar-fora");
        back.set_hexpand(true);
        back.set_vexpand(true);
        root.set_child(Some(&back));

        let col = caixa(Orientation::Vertical, 16, "menu-iniciar");
        col.set_size_request(660, -1);
        // search
        let entry = gtk::Entry::new();
        entry.add_css_class("menu-iniciar-busca");
        entry.set_placeholder_text(Some(tr("Search apps, files or ask the agent…")));
        entry.set_primary_icon_paintable(crate::conversa::ui_icon("spark", "#A5A1E2", 13).paintable().as_ref());
        // its key, at the right inside the field (as in the design)
        let campo = gtk::Overlay::new();
        campo.set_child(Some(&entry));
        let tecla = rotulo("Ctrl K", "tb-atalho");
        tecla.set_halign(Align::End);
        tecla.set_valign(Align::Center);
        tecla.set_margin_end(10);
        tecla.set_can_target(false);
        campo.add_overlay(&tecla);
        col.append(&campo);
        // pinned
        let head = caixa(Orientation::Horizontal, 0, "");
        let pinned_title = rotulo(tr("Pinned"), "menu-iniciar-titulo");
        pinned_title.set_hexpand(true);
        pinned_title.set_xalign(0.0);
        head.append(&pinned_title);
        let all = gtk::Button::new();
        all.add_css_class("menu-iniciar-todos");
        let ab = caixa(Orientation::Horizontal, 4, "");
        ab.append(&rotulo(tr("All apps"), ""));
        ab.append(&ui_icon("chevron", "#E2E2E6", 10));
        all.set_child(Some(&ab));
        head.append(&all);
        col.append(&head);
        let grid = gtk::FlowBox::new();
        grid.set_max_children_per_line(6);
        grid.set_min_children_per_line(6);
        grid.set_homogeneous(true);
        grid.set_column_spacing(4);
        grid.set_row_spacing(8);
        grid.set_selection_mode(gtk::SelectionMode::None);
        col.append(&grid);
        // recommended
        let rec_title = caixa(Orientation::Horizontal, 6, "");
        rec_title.append(&ui_icon("spark", "#A5A1E2", 13));
        rec_title.append(&rotulo(tr("Recommended by the agent"), "menu-iniciar-titulo"));
        col.append(&rec_title);
        let recommended = gtk::Grid::new();
        recommended.set_column_spacing(8);
        recommended.set_row_spacing(8);
        recommended.set_column_homogeneous(true);
        col.append(&recommended);
        let line = caixa(Orientation::Horizontal, 0, "menu-iniciar-linha");
        line.set_size_request(-1, 1);
        col.append(&line);
        // footer
        let foot = caixa(Orientation::Horizontal, 10, "");
        let avatar = caixa(Orientation::Vertical, 0, "menu-iniciar-avatar");
        avatar.set_size_request(32, 32);
        avatar.set_valign(Align::Center);
        let initial = rotulo("", "menu-iniciar-inicial");
        initial.set_vexpand(true);
        avatar.append(&initial);
        foot.append(&avatar);
        let who = caixa(Orientation::Vertical, 1, "");
        who.set_hexpand(true);
        who.set_valign(Align::Center);
        let person = rotulo("", "menu-iniciar-pessoa");
        person.set_xalign(0.0);
        let status = rotulo("", "menu-iniciar-estado");
        status.set_xalign(0.0);
        who.append(&person);
        who.append(&status);
        foot.append(&who);
        let gear = icon_button("engrenagem", tr("Settings"));
        let lock = icon_button("cadeado", tr("Lock"));
        let power = icon_button("desligar", tr("Power"));
        foot.append(&gear);
        foot.append(&lock);
        foot.append(&power);
        col.append(&foot);

        let card = Animavel::new(&col);
        card.origem(0.5, 1.0);
        card.set_halign(Align::Center);
        card.set_valign(Align::End);
        // above the taskbar (52 px) and 8 px of air
        card.set_margin_bottom(60);
        root.add_overlay(&card);
        window.set_child(Some(&root));
        let real = glib::real_name().to_string_lossy().to_string();
        let name = if real.is_empty() || real == "Unknown" { glib::user_name().to_string_lossy().to_string() } else { real };
        initial.set_text(&name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default());
        person.set_text(&name);

        let m = Rc::new(Menu { window, card, entry, grid, recommended, rec_title, status, open: Cell::new(false), shell: RefCell::new(Rc::downgrade(shell)) });
        // a click outside closes
        let click = gtk::GestureClick::new();
        let w = Rc::downgrade(&m);
        click.connect_pressed(move |_, _, x, y| {
            let Some(m) = w.upgrade() else { return };
            let inside = m.card.compute_bounds(&m.window).is_some_and(|b| b.contains_point(&gtk::graphene::Point::new(x as f32, y as f32)));
            if !inside {
                m.close();
            }
        });
        back.add_controller(click);
        let keys = gtk::EventControllerKey::new();
        let w = Rc::downgrade(&m);
        keys.connect_key_pressed(move |_, k, _, _| {
            if k == gdk::Key::Escape {
                if let Some(m) = w.upgrade() {
                    m.close();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        m.window.add_controller(keys);
        let w = Rc::downgrade(&m);
        m.entry.connect_changed(move |e| {
            if let Some(m) = w.upgrade() {
                m.fill_apps(&e.text());
            }
        });
        let w = Rc::downgrade(&m);
        m.entry.connect_activate(move |e| {
            let Some(m) = w.upgrade() else { return };
            let q = e.text().trim().to_string();
            let Some(shell) = m.shell.borrow().upgrade() else { return };
            let first = apps::buscar(&shell.todos, &q).first().map(|a| (*a).clone());
            m.close();
            match first {
                Some(a) if !q.is_empty() => {
                    let _ = apps::abrir(&a);
                }
                _ if !q.is_empty() => shell.right_bar().ask_agent(&q),
                _ => {}
            }
        });
        let (w, s) = (Rc::downgrade(&m), shell.clone());
        all.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.close();
            }
            s.grade().alternar();
        });
        let (w, s) = (Rc::downgrade(&m), shell.clone());
        gear.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.close();
            }
            s.abrir_configuracoes(None);
        });
        let (w, s) = (Rc::downgrade(&m), shell.clone());
        lock.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.close();
            }
            s.bloqueio().bloquear();
        });
        let (w, s) = (Rc::downgrade(&m), shell.clone());
        power.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.close();
            }
            s.energia().alternar();
        });
        m
    }

    /// The pinned apps, or the apps that match what is typed.
    fn fill_apps(&self, q: &str) {
        while let Some(c) = self.grid.first_child() {
            self.grid.remove(&c);
        }
        let Some(shell) = self.shell.borrow().upgrade() else { return };
        let list: Vec<App> = if q.trim().is_empty() {
            shell.fixados.borrow().iter().take(12).cloned().collect()
        } else {
            apps::buscar(&shell.todos, q.trim()).into_iter().take(12).cloned().collect()
        };
        // (with a search, the recommendations step aside)
        let searching = !q.trim().is_empty();
        self.recommended.set_visible(!searching);
        self.rec_title.set_visible(!searching);
        for a in list {
            let b = gtk::Button::new();
            b.add_css_class("menu-iniciar-app");
            let c = caixa(Orientation::Vertical, 6, "");
            c.append(&apps::imagem(&a, 48));
            let l = rotulo(&a.nome, "menu-iniciar-app-nome");
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.set_max_width_chars(12);
            c.append(&l);
            b.set_child(Some(&c));
            b.set_tooltip_text(Some(&a.nome));
            let m = self.window.downgrade();
            b.connect_clicked(move |_| {
                close();
                let _ = apps::abrir(&a);
                let _ = m;
            });
            self.grid.insert(&b, -1);
        }
    }

    /// Up to four: recent files, the requests waiting, the next event today.
    fn fill_recommended(&self) {
        while let Some(c) = self.recommended.first_child() {
            self.recommended.remove(&c);
        }
        let Some(shell) = self.shell.borrow().upgrade() else { return };
        type Tile = (&'static str, &'static str, String, String, Box<dyn Fn()>);
        let mut tiles: Vec<Tile> = Vec::new();
        if let Some(n) = shell.estado.pendentes.get().filter(|n| *n > 0) {
            let s = shell.clone();
            tiles.push(("spark", "roxo", if n == 1 { tr("Review 1 request").to_string() } else { trf("Review {n} requests", &[("n", n.to_string())]) },
                tr("the agent is waiting for you").to_string(), Box::new(move || s.right_bar().abrir(crate::lateral::Modo::Agente))));
        }
        let now_min = glib::DateTime::now_local().map(|d| d.hour() * 60 + d.minute()).unwrap_or(0) as u32;
        if let Some(ev) = crate::agenda::today().into_iter().find(|e| e.start.is_some_and(|s| s >= now_min)) {
            let s = shell.clone();
            let h = ev.start.unwrap_or(0);
            tiles.push(("relogio", "ouro", format!("{} · {:02}:{:02}", ev.title, h / 60, h % 60), tr("today").to_string(),
                Box::new(move || s.today_bar().alternar())));
        }
        let now = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
        for (p, tipo, when) in crate::arquivos::seletor::recentes_com_tipo(4) {
            if tiles.len() >= 4 {
                break;
            }
            let (icon, tone) = if tipo.starts_with("image/") { ("imagem", "verde") } else { ("doc", "coral") };
            let min = ((now - when) / 60).max(1);
            let ago = if min < 60 { trf("{n} min ago", &[("n", min.to_string())]) } else if min < 1440 { trf("{n} h ago", &[("n", (min / 60).to_string())]) } else { trf("{n} days ago", &[("n", (min / 1440).to_string())]) };
            let name = p.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
            tiles.push((icon, tone, name, ago, Box::new(move || {
                let _ = crate::apps::open_default(&gio::File::for_path(&p), gdk::Display::default().map(|x| x.app_launch_context()).as_ref().map(|c| c.upcast_ref()));
            })));
        }
        self.recommended.set_visible(!tiles.is_empty());
        self.rec_title.set_visible(!tiles.is_empty());
        for (i, (icon, tone, title, sub, act)) in tiles.into_iter().enumerate() {
            let b = gtk::Button::new();
            b.add_css_class("menu-iniciar-rec");
            let row = caixa(Orientation::Horizontal, 10, "");
            let ic = caixa(Orientation::Vertical, 0, "menu-iniciar-rec-icone");
            ic.add_css_class(tone);
            ic.set_size_request(34, 34);
            ic.set_valign(Align::Center);
            let glyph = ui_icon(icon, match tone { "roxo" => "#A5A1E2", "ouro" => "#E5C98C", "verde" => "#8ED2C3", _ => "#EFA2A6" }, 14);
            glyph.set_vexpand(true);
            ic.append(&glyph);
            row.append(&ic);
            let t = caixa(Orientation::Vertical, 1, "");
            t.set_valign(Align::Center);
            for (txt, class) in [(title.as_str(), "menu-iniciar-rec-titulo"), (sub.as_str(), "menu-iniciar-rec-sub")] {
                let l = rotulo(txt, class);
                l.set_xalign(0.0);
                l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                l.set_max_width_chars(1);
                l.set_hexpand(true);
                t.append(&l);
            }
            t.set_hexpand(true);
            row.append(&t);
            b.set_child(Some(&row));
            b.connect_clicked(move |_| {
                close();
                act();
            });
            self.recommended.attach(&b, (i % 2) as i32, (i / 2) as i32, 1, 1);
        }
    }

    fn show(self: &Rc<Self>) {
        self.open.set(true);
        self.entry.set_text("");
        self.fill_apps("");
        self.fill_recommended();
        if let Some(shell) = self.shell.borrow().upgrade() {
            let working = usize::from(shell.estado.agent_working.get()) + shell.estado.pendentes.get().unwrap_or(0).min(1);
            let tokens = crate::central_uso::mil(crate::central_uso::tokens_hoje());
            self.status.set_text(&trf("{n} agents active · {tokens} tokens today", &[("n", working.to_string()), ("tokens", tokens)]));
        }
        self.window.present();
        crate::superficies::take_keyboard(&self.window);
        self.entry.grab_focus();
        self.card.entrar((0.0, 8.0), 0.98);
        if debug() {
            eprintln!("debug menu iniciar: aberto ({} fixados)", self.grid.observe_children().n_items());
            let w = Rc::downgrade(self);
            glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
                let Some(m) = w.upgrade() else { return };
                if let Some(b) = m.card.compute_bounds(&m.window) {
                    eprintln!("debug menu iniciar: cartão em {} {} {} {}", b.x() as i32, b.y() as i32, b.width() as i32, b.height() as i32);
                }
            });
        }
    }

    fn close(&self) {
        if !self.open.replace(false) {
            return;
        }
        self.window.set_keyboard_mode(KeyboardMode::None);
        let w = self.window.downgrade();
        let c = self.card.downgrade();
        self.card.sair((0.0, 8.0), 0.98, move || {
            if let (Some(w), Some(c)) = (w.upgrade(), c.upgrade()) {
                if c.valor(Canal::Opacidade) < 0.05 {
                    w.set_visible(false);
                }
            }
        });
        let _ = mv::RAPIDA;
        if debug() {
            eprintln!("debug menu iniciar: fechado");
        }
    }
}
