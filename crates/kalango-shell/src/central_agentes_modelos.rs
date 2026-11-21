//! Agents center › Agentes › Prontos para usar (design "Galeria de modelos"): ready-made agents, teams and flows
//! shipped with the system (assets/fluxos-modelos/*.json). The featured one is drawn large with its blocks; the
//! others are cards in the columns that fit. On the right, the chosen template: how it works, what it will ask
//! permission for and where it thinks. "Usar este modelo" saves a copy through the service (agents.save, and
//! flows.save for a flow, wrapped in a flow agent) and opens it in the editor; nothing runs until the person turns it on.

use crate::central_agentes::{Go, Nav};
use crate::fluxos;
use crate::fluxos_ui::{self as ui, Segmented};
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

const TEMPLATES: &[&str] = &[
    include_str!("../assets/fluxos-modelos/faturas-do-email.json"),
    include_str!("../assets/fluxos-modelos/faxina-de-downloads.json"),
    include_str!("../assets/fluxos-modelos/fotos-do-cartao-sd.json"),
    include_str!("../assets/fluxos-modelos/resumo-da-semana.json"),
    include_str!("../assets/fluxos-modelos/triagem-da-caixa.json"),
    include_str!("../assets/fluxos-modelos/comparar-precos.json"),
    include_str!("../assets/fluxos-modelos/corrige-ate-passar.json"),
];

pub fn templates() -> Vec<Value> {
    TEMPLATES.iter().filter_map(|t| serde_json::from_str(t).ok()).collect()
}

/// A permission's level chip: (text, tone).
fn permission_level(level: &str) -> (&'static str, &'static str) {
    match level {
        "leitura" => (tr("read"), "azul"),
        "usuario" => (tr("user"), "ouro"),
        _ => (tr("asks"), "coral"),
    }
}

pub struct Gallery {
    pub body: gtk::Box,
    pub side: gtk::Box,
    pub nav: RefCell<Option<Nav>>,
    pub on_pick: RefCell<Option<Rc<dyn Fn()>>>,
    query: gtk::Entry,
    category: RefCell<String>,
    featured: gtk::Box,
    grid: gtk::FlowBox,
    chosen: RefCell<String>,
    s_tile: gtk::Box,
    s_name: gtk::Label,
    s_sub: gtk::Label,
    s_intro: gtk::Label,
    s_steps: gtk::Box,
    s_perms: gtk::Box,
    #[allow(dead_code)]
    s_model: gtk::DropDown,
    s_error: gtk::Label,
}

impl Gallery {
    pub fn new(panel_width: i32) -> Rc<Gallery> {
        let body = caixa(Orientation::Vertical, 0, "fx-corpo");
        body.set_hexpand(true);
        let bar = caixa(Orientation::Horizontal, 14, "fx-filtros");
        let (sb, query) = ui::search(tr("Search templates…"));
        sb.set_size_request(260, -1);
        sb.set_hexpand(false);
        bar.append(&sb);
        let me: Rc<RefCell<Weak<Gallery>>> = Rc::default();
        let m2 = me.clone();
        let seg = Segmented::new(
            &[("all".into(), tr("All").into()), ("arquivos".into(), tr("Files").into()), ("email".into(), tr("E-mail and calendar").into()), ("escrita".into(), tr("Writing").into()), ("sistema".into(), tr("System").into()), ("pesquisa".into(), tr("Research").into())],
            "all",
            move |id| {
                if let Some(g) = m2.borrow().upgrade() {
                    *g.category.borrow_mut() = id.to_string();
                    g.render();
                }
            },
        );
        seg.root.add_css_class("fx-seg-filtros");
        bar.append(&seg.root);
        body.append(&bar);
        let featured = caixa(Orientation::Vertical, 0, "");
        let grid = gtk::FlowBox::new();
        grid.add_css_class("fx-grade");
        grid.set_selection_mode(gtk::SelectionMode::None);
        grid.set_homogeneous(true);
        grid.set_column_spacing(13);
        grid.set_row_spacing(13);
        grid.set_min_children_per_line(1);
        grid.set_max_children_per_line(4);
        grid.set_valign(Align::Start);
        let col = caixa(Orientation::Vertical, 13, "");
        col.append(&featured);
        col.append(&grid);
        body.append(&ui::scroll(&col));

        let side = ui::side_panel(panel_width);
        let pt = caixa(Orientation::Horizontal, 8, "");
        pt.append(&crate::conversa::ui_icon("estrela-n", ui::ACCENT, 14));
        pt.append(&rotulo(tr("SELECTED TEMPLATE"), "central-painel-agente"));
        side.append(&pt);
        let inner = caixa(Orientation::Vertical, 0, "");
        let head = caixa(Orientation::Horizontal, 12, "");
        head.set_margin_top(16);
        let s_tile = caixa(Orientation::Horizontal, 0, "");
        head.append(&s_tile);
        let hc = caixa(Orientation::Vertical, 2, "");
        hc.set_hexpand(true);
        hc.set_valign(Align::Center);
        let s_name = ui::cut("", "fx-nome-grande");
        hc.append(&s_name);
        let s_sub = ui::cut("", "central-agente-sub");
        hc.append(&s_sub);
        head.append(&hc);
        inner.append(&head);
        let s_intro = ui::wrapped("", "fx-descricao");
        s_intro.set_margin_top(14);
        inner.append(&s_intro);
        let h1 = ui::section(tr("HOW IT WORKS"));
        h1.set_margin_top(14);
        inner.append(&h1);
        let s_steps = caixa(Orientation::Vertical, 0, "fx-ficha");
        s_steps.set_margin_top(10);
        inner.append(&s_steps);
        let h2 = ui::section(tr("WILL ASK PERMISSION TO"));
        h2.set_margin_top(20);
        inner.append(&h2);
        let s_perms = caixa(Orientation::Vertical, 0, "fx-ficha");
        s_perms.set_margin_top(10);
        inner.append(&s_perms);
        let s_model = ui::dropdown(&[tr("LFM3 · local (automatic by input)").to_string()], 0);
        let wm = ui::labelled(tr("Where it thinks"), &s_model);
        wm.set_margin_top(20);
        inner.append(&wm);
        side.append(&ui::scroll(&inner));
        let s_error = ui::error_label();
        side.append(&s_error);
        let buttons = caixa(Orientation::Horizontal, 8, "fx-botoes-painel");
        buttons.set_homogeneous(true);
        let try_b = ui::button(Some("olho"), tr("Try it in the editor"), false);
        let use_b = ui::button(Some("check"), tr("Use this template"), true);
        buttons.append(&try_b);
        buttons.append(&use_b);
        side.append(&buttons);

        let g = Rc::new(Gallery {
            body, side, nav: RefCell::default(), on_pick: RefCell::default(), query: query.clone(), category: RefCell::new("all".into()), featured, grid, chosen: RefCell::new("faturas-do-email".into()),
            s_tile, s_name, s_sub, s_intro, s_steps, s_perms, s_model, s_error,
        });
        *me.borrow_mut() = Rc::downgrade(&g);
        let w = Rc::downgrade(&g);
        query.connect_changed(move |_| {
            if let Some(g) = w.upgrade() {
                g.render();
            }
        });
        let w = Rc::downgrade(&g);
        use_b.connect_clicked(move |_| {
            if let Some(g) = w.upgrade() {
                let id = g.chosen.borrow().clone();
                g.use_template(&id, false);
            }
        });
        let w = Rc::downgrade(&g);
        try_b.connect_clicked(move |_| {
            if let Some(g) = w.upgrade() {
                let id = g.chosen.borrow().clone();
                g.use_template(&id, true);
            }
        });
        g
    }

    pub fn set_width(&self, _width: i32) {}

    fn go(&self, g: Go) {
        if let Some(n) = self.nav.borrow().clone() {
            n(g);
        }
    }

    fn chosen_template(&self) -> Option<Value> {
        let c = self.chosen.borrow().clone();
        templates().into_iter().find(|t| t["id"] == c.as_str())
    }

    pub fn render(self: &Rc<Self>) {
        let q = self.query.text().to_string();
        let cat = self.category.borrow().clone();
        let all: Vec<Value> = templates()
            .into_iter()
            .filter(|t| cat == "all" || t["category"] == cat.as_str())
            .filter(|t| ui::matches_query(&q, &[t["name"].as_str().unwrap_or(""), t["summary"].as_str().unwrap_or(""), t["chip"].as_str().unwrap_or("")]))
            .collect();
        ui::clear_box(&self.featured);
        ui::clear(&self.grid);
        for t in &all {
            if t["featured"].as_bool().unwrap_or(false) {
                self.featured.append(&self.featured_card(t));
            } else {
                let c = self.card(t);
                c.set_size_request(250, -1);
                self.grid.insert(&c, -1);
            }
        }
        if all.is_empty() {
            self.featured.append(&ui::wrapped(tr("No template matches."), "central-sub"));
        }
        self.render_side();
    }

    fn pick(self: &Rc<Self>, id: &str) {
        *self.chosen.borrow_mut() = id.to_string();
        self.render();
        if let Some(f) = self.on_pick.borrow().clone() {
            f();
        }
    }

    fn clickable(self: &Rc<Self>, w: &gtk::Box, id: &str) {
        let click = gtk::GestureClick::new();
        let (g, id) = (Rc::downgrade(self), id.to_string());
        click.connect_released(move |_, _, _, _| {
            if let Some(g) = g.upgrade() {
                g.pick(&id);
            }
        });
        w.add_controller(click);
    }

    fn featured_card(self: &Rc<Self>, t: &Value) -> gtk::Box {
        let id = t["id"].as_str().unwrap_or("");
        let c = caixa(Orientation::Horizontal, 18, "fx-destaque");
        if *self.chosen.borrow() == id {
            c.add_css_class("escolhido");
        }
        let left = caixa(Orientation::Vertical, 0, "");
        left.set_hexpand(true);
        let tags = caixa(Orientation::Horizontal, 6, "");
        tags.append(&ui::chip(ui::kind_label(t["kind"].as_str().unwrap_or("")), "teal"));
        tags.append(&ui::chip(tr("runs on the computer"), ""));
        left.append(&tags);
        let n = ui::wrapped(t["name"].as_str().unwrap_or(""), "fx-destaque-nome");
        n.set_margin_top(8);
        left.append(&n);
        let d = ui::wrapped(t["summary"].as_str().unwrap_or(""), "fx-descricao");
        d.set_margin_top(6);
        left.append(&d);
        let meta: Vec<String> = t["meta"].as_array().into_iter().flatten().filter_map(|m| m.as_str().map(str::to_string)).collect();
        let m = ui::wrapped(&meta.join("  ·  "), "central-agente-sub");
        m.set_margin_top(6);
        left.append(&m);
        c.append(&left);
        // the blocks, as the builder draws them
        let blocks = gtk::Grid::new();
        blocks.add_css_class("fx-blocos");
        blocks.set_column_spacing(10);
        blocks.set_row_spacing(10);
        blocks.set_valign(Align::Center);
        let tones = ["ouro", "acento", "azul", "cinza", "lilas", "teal"];
        let n = t["flow"]["nodes"].as_array().map(|a| a.len()).unwrap_or(3).clamp(3, 6);
        for i in 0..n {
            let b = caixa(Orientation::Horizontal, 0, "fx-bloco");
            b.set_size_request(50, 20);
            let dot = caixa(Orientation::Horizontal, 0, "fx-bloco-ponto");
            dot.add_css_class(tones[i % tones.len()]);
            dot.set_valign(Align::Center);
            b.append(&dot);
            let (col, row) = if i < 4 { (i as i32, 0) } else { (2, (i - 3) as i32) };
            blocks.attach(&b, col, row, 1, 1);
        }
        c.append(&blocks);
        let use_b = ui::button(None, tr("Use this"), true);
        use_b.set_valign(Align::Start);
        let (w, tid) = (Rc::downgrade(self), id.to_string());
        use_b.connect_clicked(move |_| {
            if let Some(g) = w.upgrade() {
                g.use_template(&tid, false);
            }
        });
        c.append(&use_b);
        self.clickable(&c, id);
        crate::central::posicao_debug(&format!("modelo {id}"), c.upcast_ref());
        c
    }

    fn card(self: &Rc<Self>, t: &Value) -> gtk::Box {
        let id = t["id"].as_str().unwrap_or("");
        let c = caixa(Orientation::Vertical, 0, "fx-cartao");
        c.add_css_class("modelo");
        if *self.chosen.borrow() == id {
            c.add_css_class("escolhido");
        }
        let top = caixa(Orientation::Horizontal, 12, "");
        top.append(&ui::icon_tile(t["icon"].as_str().unwrap_or("spark"), t["tone"].as_str().unwrap_or("acento"), 32));
        let tc = caixa(Orientation::Vertical, 1, "");
        tc.set_hexpand(true);
        tc.append(&ui::cut(t["name"].as_str().unwrap_or(""), "fx-cartao-nome"));
        tc.append(&ui::cut(ui::kind_label(t["kind"].as_str().unwrap_or("")), "fx-cartao-tipo"));
        top.append(&tc);
        c.append(&top);
        let d = ui::wrapped(t["summary"].as_str().unwrap_or(""), "fx-cartao-desc");
        d.set_lines(2);
        d.set_ellipsize(gtk::pango::EllipsizeMode::End);
        d.set_margin_top(10);
        c.append(&d);
        let sp = caixa(Orientation::Vertical, 0, "");
        sp.set_vexpand(true);
        c.append(&sp);
        let bottom = caixa(Orientation::Horizontal, 6, "");
        bottom.set_margin_top(8);
        bottom.append(&ui::chip(t["chip"].as_str().unwrap_or(""), ""));
        let s = caixa(Orientation::Horizontal, 0, "");
        s.set_hexpand(true);
        bottom.append(&s);
        let use_b = ui::link(tr("Use"));
        let (w, tid) = (Rc::downgrade(self), id.to_string());
        use_b.connect_clicked(move |_| {
            if let Some(g) = w.upgrade() {
                g.use_template(&tid, false);
            }
        });
        bottom.append(&use_b);
        c.append(&bottom);
        self.clickable(&c, id);
        c
    }

    fn render_side(&self) {
        let Some(t) = self.chosen_template() else { return };
        ui::clear_box(&self.s_tile);
        self.s_tile.append(&ui::icon_tile(t["icon"].as_str().unwrap_or("spark"), t["tone"].as_str().unwrap_or("acento"), 44));
        self.s_name.set_text(t["name"].as_str().unwrap_or(""));
        self.s_sub.set_text(&format!("{} · {} · {}", ui::kind_label(t["kind"].as_str().unwrap_or("")), tr("made by KalangoOS"), trf("version {n}", &[("n", t["version"].as_u64().unwrap_or(1).to_string())])));
        self.s_intro.set_text(t["intro"].as_str().unwrap_or(""));
        ui::clear_box(&self.s_steps);
        for (i, s) in t["steps"].as_array().into_iter().flatten().enumerate() {
            let r = caixa(Orientation::Horizontal, 10, "fx-ficha-linha");
            r.append(&ui::left(&(i + 1).to_string(), "fx-passo-numero"));
            r.append(&ui::wrapped(s.as_str().unwrap_or(""), "fx-ficha-valor-l"));
            self.s_steps.append(&r);
        }
        ui::clear_box(&self.s_perms);
        for p in t["permissions"].as_array().into_iter().flatten() {
            let r = caixa(Orientation::Horizontal, 10, "fx-ficha-linha");
            r.append(&crate::conversa::ui_icon(p["icon"].as_str().unwrap_or("info"), ui::MUTED, 14));
            r.append(&ui::wrapped(p["text"].as_str().unwrap_or(""), "fx-ficha-valor-l"));
            let (l, tone) = permission_level(p["level"].as_str().unwrap_or(""));
            r.append(&ui::chip(l, tone));
            self.s_perms.append(&r);
        }
    }

    /// Saves a copy of the template (sub-agents, the flow, the main agent) and opens it in the editor.
    fn use_template(self: &Rc<Self>, id: &str, try_it: bool) {
        let Some(t) = templates().into_iter().find(|t| t["id"] == id) else { return };
        let (agents, flow) = ui::instantiate_template(&t, |_| ui::new_id("agente"));
        let name = t["name"].as_str().unwrap_or("").to_string();
        ui::show_error(&self.s_error, "");
        let w = Rc::downgrade(self);
        // in order: each agent (the main one last), then the flow and its agent
        std::thread::spawn({
            let (tx, rx) = std::sync::mpsc::channel::<Result<Value, String>>();
            let agents2 = agents.clone();
            let name2 = name.clone();
            glib_wait(rx, move |r| {
                let Some(g) = w.upgrade() else { return };
                match r {
                    Ok(main) => {
                        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                            eprintln!("debug central: modelo {name2} salvo");
                        }
                        g.go(Go::Agent { agent: main, new: false, chat: try_it });
                    }
                    Err(e) => ui::show_error(&g.s_error, &trf("Could not save the template: {error}", &[("error", e)])),
                }
            });
            move || {
                let timeout = std::time::Duration::from_secs(20);
                let mut main = Value::Null;
                for a in &agents2 {
                    if let Err(e) = fluxos::call_blocking("agents.save", &json!({"agent": a}), timeout) {
                        let _ = tx.send(Err(e));
                        return;
                    }
                    main = a.clone();
                }
                if let Some(f) = flow {
                    let r = fluxos::call_blocking("flows.save", &json!({"name": name, "flow": f}), timeout);
                    let fid = match r {
                        Ok(v) => v["id"].as_str().unwrap_or("").to_string(),
                        Err(e) => {
                            let _ = tx.send(Err(e));
                            return;
                        }
                    };
                    let wrapper = json!({"id": ui::new_id("agente"), "name": name, "description": t["summary"], "kind": "flow", "config": {"flow_id": fid}});
                    if let Err(e) = fluxos::call_blocking("agents.save", &json!({"agent": wrapper}), timeout) {
                        let _ = tx.send(Err(e));
                        return;
                    }
                    main = wrapper;
                }
                let _ = tx.send(Ok(main));
            }
        });
    }
}

/// Runs `done` on the main loop with what the worker sends.
pub fn glib_wait<T: 'static>(rx: std::sync::mpsc::Receiver<T>, done: impl FnOnce(T) + 'static) {
    let mut done = Some(done);
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(40), move || match rx.try_recv() {
        Ok(v) => {
            if let Some(d) = done.take() {
                d(v);
            }
            gtk::glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => gtk::glib::ControlFlow::Continue,
        Err(_) => gtk::glib::ControlFlow::Break,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_is_complete() {
        let all = templates();
        assert_eq!(all.len(), TEMPLATES.len(), "every file is valid JSON");
        assert_eq!(all.iter().filter(|t| t["featured"] == true).count(), 1);
        for t in &all {
            for k in ["id", "name", "kind", "category", "summary", "intro", "chip"] {
                assert!(t[k].as_str().is_some_and(|s| !s.is_empty()), "{} sem {k}", t["id"]);
            }
            assert!(!t["steps"].as_array().unwrap().is_empty());
            let has_main = t["agents"].as_array().unwrap().iter().any(|a| a["id"] == "$main");
            assert!(has_main || t["flow"].is_object(), "{}: an agent or a flow", t["id"]);
            for a in t["agents"].as_array().unwrap() {
                assert!(a["id"].as_str().unwrap().starts_with('$'), "{}: template ids start with $", t["id"]);
            }
            let (agents, _) = ui::instantiate_template(t, |x| format!("novo{x}"));
            assert!(agents.iter().all(|a| !a.to_string().contains("\"$")), "{}: every id replaced", t["id"]);
        }
    }

    #[test]
    fn permission_chips() {
        assert_eq!(permission_level("leitura").1, "azul");
        assert_eq!(permission_level("pergunta").1, "coral");
    }
}
