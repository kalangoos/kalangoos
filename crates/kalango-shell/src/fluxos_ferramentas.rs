//! Agents center › Agentes › Ferramentas (design "Agentes · Ferramentas"): every tool an agent can use — the system's
//! (kalangod, with its level: the level decides who approves), the person's own (HTTP and imported from OpenAPI)
//! and the connectors' (MCP, Ajustes › Conectores) — with who uses each and its permission (livre, perguntar, nunca).
//! A custom tool's permission is saved with it (tools.save_http); a system or connector tool's is the choice of
//! every agent that uses it, changed in each of them (agents.save). On the right, the chosen tool.

use crate::central_agentes::{Go, Nav};
use crate::fluxos;
use crate::fluxos_ui::{self as ui, Segmented};
use crate::superficies::caixa;
use gtk::prelude::*;
use gtk::{Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// kalangod's tools for its own undo: no agent is offered them.
const INTERNAL: [&str; 2] = ["arquivos.restaurar", "arquivos.apagar_criado"];

/// One row: a system, custom or connector tool.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolItem {
    pub name: String,
    pub description: String,
    /// "system", "custom", "connector"
    pub origin: &'static str,
    pub level: String,
    /// custom: its row from tools.list
    pub row: Value,
}

/// The groups of the list from tools.list and the installed connectors.
pub fn classify(list: &Value, connectors: &[String]) -> Vec<ToolItem> {
    let mut v = Vec::new();
    for s in list["system"].as_array().into_iter().flatten() {
        let name = s["name"].as_str().unwrap_or("").to_string();
        if INTERNAL.contains(&name.as_str()) {
            continue;
        }
        let origin = if connectors.iter().any(|c| c == ui::tool_prefix(&name)) { "connector" } else { "system" };
        v.push(ToolItem { description: s["description"].as_str().unwrap_or("").to_string(), level: s["level"].as_str().unwrap_or("").to_string(), name, origin, row: s.clone() });
    }
    for c in list["custom"].as_array().into_iter().flatten() {
        v.push(ToolItem {
            name: c["name"].as_str().unwrap_or("").to_string(),
            description: c["config"]["description"].as_str().unwrap_or("").to_string(),
            origin: "custom",
            level: c["kind"].as_str().unwrap_or("http").to_string(),
            row: c.clone(),
        });
    }
    v
}

/// The agents (names) that use a tool, and the permission they agree on (None when they differ or none uses it).
pub fn users(agents: &[Value], tool: &str) -> (Vec<String>, Option<String>) {
    let mut names = Vec::new();
    let mut perms: Vec<String> = Vec::new();
    for a in agents {
        for t in a["config"]["tools"].as_array().into_iter().flatten() {
            if t["name"].as_str() == Some(tool) {
                names.push(a["name"].as_str().unwrap_or("").to_string());
                perms.push(t["permission"].as_str().unwrap_or("ask").to_string());
            }
        }
    }
    let agreed = perms.first().filter(|p| perms.iter().all(|x| x == *p)).cloned();
    (names, agreed)
}

/// "Planejador", "3 agentes", "nenhum".
pub fn users_label(names: &[String]) -> String {
    match names.len() {
        0 => tr("none").to_string(),
        1 => names[0].clone(),
        n => trf("{n} agents", &[("n", n.to_string())]),
    }
}

/// A system tool's glyph by its prefix.
fn tool_glyph(name: &str) -> &'static str {
    match ui::tool_prefix(name) {
        "arquivos" => "pasta",
        "pacotes" | "atualizacoes" | "tarefas" | "processos" | "sistema" => "terminal",
        "rede" | "firewall" => "wifi",
        "web" => "globo",
        "linha_do_tempo" | "pontos" => "relogio",
        "discos" => "disco",
        "usuarios" => "usuario",
        "notificar" | "notificacoes" => "sino",
        _ => "codigo",
    }
}

pub struct ToolsTab {
    pub body: gtk::Box,
    pub side: gtk::Box,
    pub nav: RefCell<Option<Nav>>,
    pub on_pick: RefCell<Option<Rc<dyn Fn()>>>,
    query: gtk::Entry,
    filter: RefCell<String>,
    seg: Segmented,
    list: gtk::Box,
    items: RefCell<Vec<ToolItem>>,
    agents: RefCell<Vec<Value>>,
    chosen: RefCell<String>,
    loading: Cell<u32>,
    error: gtk::Label,
    s_title: gtk::Box,
    s_desc: gtk::Label,
    s_request: gtk::Label,
    s_facts: gtk::Box,
    s_users: gtk::FlowBox,
    s_buttons: gtk::Box,
    s_test: gtk::Button,
    s_dup: gtk::Button,
    s_edit: gtk::Button,
}

impl ToolsTab {
    pub fn new(panel_width: i32) -> Rc<ToolsTab> {
        let body = caixa(Orientation::Vertical, 0, "fx-corpo");
        body.set_hexpand(true);
        let bar = caixa(Orientation::Horizontal, 14, "fx-filtros");
        let (sb, query) = ui::search(tr("Search tools…"));
        sb.set_size_request(240, -1);
        sb.set_hexpand(false);
        bar.append(&sb);
        let me: Rc<RefCell<Weak<ToolsTab>>> = Rc::default();
        let m2 = me.clone();
        let seg = Segmented::new(
            &[("all".into(), tr("All").into()), ("system".into(), tr("System").into()), ("custom".into(), tr("HTTP / API").into()), ("connector".into(), tr("Connectors").into())],
            "all",
            move |id| {
                if let Some(t) = m2.borrow().upgrade() {
                    *t.filter.borrow_mut() = id.to_string();
                    t.render();
                }
            },
        );
        seg.root.add_css_class("fx-seg-filtros");
        bar.append(&seg.root);
        body.append(&bar);
        let error = ui::error_label();
        body.append(&error);
        let list = caixa(Orientation::Vertical, 8, "");
        list.set_valign(Align::Start);
        body.append(&ui::scroll(&list));

        let side = ui::side_panel(panel_width);
        side.append(&ui::panel_title("codigo", tr("SELECTED TOOL")));
        let inner = caixa(Orientation::Vertical, 0, "");
        let s_title = caixa(Orientation::Horizontal, 12, "");
        s_title.set_margin_top(16);
        inner.append(&s_title);
        let s_desc = ui::wrapped("", "fx-descricao");
        s_desc.set_margin_top(14);
        inner.append(&s_desc);
        let s_request = ui::wrapped("", "fx-codigo");
        s_request.set_selectable(true);
        s_request.set_margin_top(14);
        inner.append(&s_request);
        let s_facts = caixa(Orientation::Vertical, 0, "fx-ficha");
        s_facts.set_margin_top(14);
        inner.append(&s_facts);
        let h = ui::section(tr("WHO USES IT"));
        h.set_margin_top(20);
        inner.append(&h);
        let s_users = gtk::FlowBox::new();
        s_users.add_css_class("fx-chips");
        s_users.set_selection_mode(gtk::SelectionMode::None);
        s_users.set_column_spacing(6);
        s_users.set_row_spacing(6);
        s_users.set_max_children_per_line(30);
        s_users.set_margin_top(10);
        inner.append(&s_users);
        side.append(&ui::scroll(&inner));
        let s_buttons = caixa(Orientation::Horizontal, 8, "fx-botoes-painel");
        s_buttons.set_homogeneous(true);
        let s_test = ui::button(Some("tocar"), tr("Test"), false);
        let s_dup = ui::button(Some("compartilhar"), tr("Make a copy"), false);
        let s_edit = ui::button(Some("ajustes"), tr("Edit"), true);
        s_buttons.append(&s_test);
        s_buttons.append(&s_dup);
        s_buttons.append(&s_edit);
        side.append(&s_buttons);

        let t = Rc::new(ToolsTab {
            body, side, nav: RefCell::default(), on_pick: RefCell::default(), query: query.clone(), filter: RefCell::new("all".into()), seg, list, items: RefCell::default(), agents: RefCell::default(),
            chosen: RefCell::default(), loading: Cell::new(0), error, s_title, s_desc, s_request, s_facts, s_users, s_buttons, s_test: s_test.clone(), s_dup: s_dup.clone(), s_edit: s_edit.clone(),
        });
        *me.borrow_mut() = Rc::downgrade(&t);
        let w = Rc::downgrade(&t);
        query.connect_changed(move |_| {
            if let Some(t) = w.upgrade() {
                t.render();
            }
        });
        let w = Rc::downgrade(&t);
        s_edit.connect_clicked(move |_| {
            if let Some((t, it)) = w.upgrade().and_then(|t| t.current().map(|i| (t, i))) {
                t.go(Go::Tool { row: Some(it.row), test: false });
            }
        });
        let w = Rc::downgrade(&t);
        s_test.connect_clicked(move |_| {
            if let Some((t, it)) = w.upgrade().and_then(|t| t.current().map(|i| (t, i))) {
                t.go(Go::Tool { row: Some(it.row), test: true });
            }
        });
        let w = Rc::downgrade(&t);
        s_dup.connect_clicked(move |_| {
            if let Some((t, it)) = w.upgrade().and_then(|t| t.current().map(|i| (t, i))) {
                t.duplicate(&it);
            }
        });
        t
    }

    fn go(&self, g: Go) {
        if let Some(n) = self.nav.borrow().clone() {
            n(g);
        }
    }

    fn current(&self) -> Option<ToolItem> {
        let c = self.chosen.borrow().clone();
        self.items.borrow().iter().find(|i| i.name == c).cloned()
    }

    pub fn reload(self: &Rc<Self>) {
        self.loading.set(2);
        let w = Rc::downgrade(self);
        fluxos::call("tools.list", json!({}), move |r| {
            let Some(t) = w.upgrade() else { return };
            match r {
                Ok(v) => {
                    let connectors: Vec<String> = ui::connectors().into_iter().map(|c| c.0).collect();
                    *t.items.borrow_mut() = classify(&v, &connectors);
                    ui::show_error(&t.error, "");
                }
                Err(e) => ui::show_error(&t.error, &trf("The agents service does not answer ({error}).", &[("error", e)])),
            }
            t.loaded();
        });
        let w = Rc::downgrade(self);
        fluxos::call("agents.list", json!({}), move |r| {
            let Some(t) = w.upgrade() else { return };
            *t.agents.borrow_mut() = r.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default().into_iter().map(|x| x["agent"].clone()).collect();
            t.loaded();
        });
    }

    fn loaded(self: &Rc<Self>) {
        self.loading.set(self.loading.get().saturating_sub(1));
        if self.loading.get() > 0 {
            return;
        }
        if self.current().is_none() {
            let first = self.items.borrow().iter().find(|i| i.origin == "custom").or(self.items.borrow().first()).map(|i| i.name.clone()).unwrap_or_default();
            *self.chosen.borrow_mut() = first;
        }
        self.render();
    }

    fn render(self: &Rc<Self>) {
        let items = self.items.borrow().clone();
        self.seg.set_label("all", &trf("All {n}", &[("n", items.len().to_string())]));
        let q = self.query.text().to_string();
        let f = self.filter.borrow().clone();
        ui::clear_box(&self.list);
        let chosen = self.chosen.borrow().clone();
        for (origin, title, sub) in [
            ("system", tr("FROM THE SYSTEM"), tr("from kalangod · the level decides who approves")),
            ("custom", tr("CREATED BY YOU"), tr("HTTP and imported from OpenAPI")),
            ("connector", tr("FROM CONNECTORS"), tr("from the MCP connectors in Settings › Connectors")),
        ] {
            if f != "all" && f != origin {
                continue;
            }
            let group: Vec<&ToolItem> = items.iter().filter(|i| i.origin == origin && ui::matches_query(&q, &[&i.name, &i.description])).collect();
            if group.is_empty() {
                continue;
            }
            let h = caixa(Orientation::Horizontal, 8, "fx-grupo");
            h.append(&ui::left(title, "fx-grupo-titulo"));
            h.append(&ui::cut(sub, "fx-grupo-sub"));
            self.list.append(&h);
            for it in group {
                self.list.append(&self.row(it, it.name == chosen));
            }
        }
        if self.list.first_child().is_none() {
            let l = ui::wrapped(if items.is_empty() { tr("No tool yet: kalangod's appear when it runs; create your own with “New tool” or “Import OpenAPI”.") } else { tr("No tool matches.") }, "central-sub");
            self.list.append(&l);
        }
        self.render_side();
    }

    fn row(self: &Rc<Self>, it: &ToolItem, chosen: bool) -> gtk::Box {
        let r = caixa(Orientation::Horizontal, 12, "fx-ferramenta");
        if chosen {
            r.add_css_class("escolhido");
        }
        if it.origin == "custom" {
            let m = it.row["config"]["method"].as_str().unwrap_or("GET").to_uppercase();
            let mc = ui::mono_chip(&m, ui::method_tone(&m));
            mc.add_css_class("metodo");
            r.append(&mc);
        } else {
            r.append(&ui::icon_tile(if it.origin == "connector" { "enviar" } else { tool_glyph(&it.name) }, if it.origin == "connector" { "lilas" } else { ui::level_tone(&it.level) }, 28));
        }
        let col = caixa(Orientation::Vertical, 2, "");
        col.set_hexpand(true);
        col.set_valign(Align::Center);
        let top = caixa(Orientation::Horizontal, 8, "");
        let n = ui::left(&it.name, "fx-ferramenta-nome");
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        top.append(&n);
        let (tag, tone) = match it.origin {
            "custom" => (if it.level == "openapi" { tr("OpenAPI") } else { "HTTP" }, if it.level == "openapi" { "lilas" } else { "" }),
            "connector" => (tr("connector"), "lilas"),
            _ => (ui::level_label(&it.level), ui::level_tone(&it.level)),
        };
        top.append(&ui::chip(tag, tone));
        col.append(&top);
        col.append(&ui::cut(&it.description, "fx-ferramenta-desc"));
        r.append(&col);
        let (names, agreed) = users(&self.agents.borrow(), &it.name);
        let who = ui::cut(&users_label(&names), "fx-ferramenta-quem");
        who.set_xalign(1.0);
        who.set_hexpand(false);
        who.set_max_width_chars(18);
        who.set_valign(Align::Start);
        r.append(&who);
        let current = if it.origin == "custom" { it.row["permission"].as_str().unwrap_or("ask").to_string() } else { agreed.clone().unwrap_or_default() };
        let (w, item) = (Rc::downgrade(self), it.clone());
        let seg = Segmented::new(&[("free".into(), tr("free").into()), ("ask".into(), tr("ask").into()), ("never".into(), tr("never").into())], &current, move |p| {
            if let Some(t) = w.upgrade() {
                t.set_permission(&item, p);
            }
        });
        seg.root.add_css_class("fx-seg-permissao");
        seg.root.add_css_class(&format!("p-{current}"));
        seg.root.set_valign(Align::Center);
        if it.origin != "custom" && names.is_empty() {
            seg.root.set_sensitive(false);
            seg.root.set_tooltip_text(Some(tr("No agent uses it: choose it in an agent's tools")));
        }
        r.append(&seg.root);
        let click = gtk::GestureClick::new();
        let (w, name) = (Rc::downgrade(self), it.name.clone());
        click.connect_released(move |_, _, _, _| {
            let Some(t) = w.upgrade() else { return };
            *t.chosen.borrow_mut() = name.clone();
            t.render();
            let pick = t.on_pick.borrow().clone();
            if let Some(f) = pick {
                f();
            }
        });
        r.add_controller(click);
        crate::central::posicao_debug(&format!("ferramenta {}", it.name), r.upcast_ref());
        r
    }

    fn set_permission(self: &Rc<Self>, it: &ToolItem, p: &str) {
        if it.origin == "custom" {
            let w = Rc::downgrade(self);
            fluxos::call("tools.save_http", json!({"id": it.row["id"], "kind": it.row["kind"], "tool": it.row["config"], "permission": p}), move |r| {
                if let Some(t) = w.upgrade() {
                    if let Err(e) = r {
                        ui::show_error(&t.error, &e);
                    }
                    t.reload();
                }
            });
            return;
        }
        // every agent that uses it
        let agents: Vec<Value> = self.agents.borrow().iter().filter(|a| a["config"]["tools"].as_array().into_iter().flatten().any(|t| t["name"].as_str() == Some(it.name.as_str()))).cloned().collect();
        let n = agents.len();
        let left = Rc::new(Cell::new(n));
        for mut a in agents {
            if let Some(tools) = a["config"]["tools"].as_array_mut() {
                for t in tools.iter_mut().filter(|t| t["name"].as_str() == Some(it.name.as_str())) {
                    t["permission"] = json!(p);
                }
            }
            let (w, left) = (Rc::downgrade(self), left.clone());
            fluxos::call("agents.save", json!({"agent": a}), move |r| {
                let Some(t) = w.upgrade() else { return };
                if let Err(e) = r {
                    ui::show_error(&t.error, &e);
                }
                left.set(left.get() - 1);
                if left.get() == 0 {
                    t.reload();
                }
            });
        }
    }

    fn duplicate(self: &Rc<Self>, it: &ToolItem) {
        if it.origin != "custom" {
            return;
        }
        let mut tool = it.row["config"].clone();
        tool["name"] = json!(format!("{}_copia", it.name));
        let w = Rc::downgrade(self);
        fluxos::call("tools.save_http", json!({"kind": it.row["kind"], "tool": tool, "permission": it.row["permission"]}), move |r| {
            if let Some(t) = w.upgrade() {
                if let Err(e) = r {
                    ui::show_error(&t.error, &e);
                }
                t.reload();
            }
        });
    }

    fn render_side(&self) {
        let Some(it) = self.current() else {
            self.s_buttons.set_visible(false);
            return;
        };
        ui::clear_box(&self.s_title);
        let custom = it.origin == "custom";
        let cfg = &it.row["config"];
        if custom {
            let m = cfg["method"].as_str().unwrap_or("GET").to_uppercase();
            let mc = ui::mono_chip(&m, ui::method_tone(&m));
            mc.add_css_class("metodo");
            mc.set_valign(Align::Start);
            mc.set_margin_top(4);
            self.s_title.append(&mc);
        } else {
            self.s_title.append(&ui::icon_tile(tool_glyph(&it.name), ui::level_tone(&it.level), 44));
        }
        let col = caixa(Orientation::Vertical, 2, "");
        col.set_hexpand(true);
        col.append(&ui::cut(&it.name, "fx-nome-grande-mono"));
        let sub = match it.origin {
            "custom" => format!("{} · {}", if it.level == "openapi" { tr("imported from OpenAPI") } else { "HTTP" }, if ui::made_by_agent(it.row["created_by"].as_str().unwrap_or("")) { tr("created by the agent") } else { tr("created by you") }),
            "connector" => format!("{} · {}", tr("connector"), ui::tool_prefix(&it.name)),
            _ => format!("{} · {}", tr("kalangod"), trf("level {level}", &[("level", ui::level_label(&it.level).to_string())])),
        };
        col.append(&ui::cut(&sub, "central-agente-sub"));
        self.s_title.append(&col);
        self.s_desc.set_text(&trf("What the model reads: “{text}”", &[("text", it.description.clone())]));
        ui::clear_box(&self.s_facts);
        let mut facts: Vec<(String, String)> = Vec::new();
        if custom {
            let mut req = format!("{} {}", cfg["method"].as_str().unwrap_or("GET").to_uppercase(), cfg["endpoint"].as_str().unwrap_or(""));
            for (k, v) in cfg["headers"].as_object().into_iter().flatten() {
                req.push_str(&format!("\n{k}: {}", v.as_str().unwrap_or("")));
            }
            self.s_request.set_text(&req);
            let params: Vec<String> = cfg["parameters"].as_array().into_iter().flatten().map(|p| {
                let mut s = format!("{} ({}", p["name"].as_str().unwrap_or(""), ui::param_type_label(p["type"].as_str().unwrap_or("string"), p["fixed"].is_array()));
                if p["required"].as_bool().unwrap_or(false) {
                    s.push_str(&format!(", {}", tr("required")));
                }
                s.push(')');
                s
            }).collect();
            facts.push((tr("Parameters").into(), if params.is_empty() { tr("none").into() } else { params.join(" · ") }));
            let outs: Vec<String> = cfg["outputs"].as_array().into_iter().flatten().map(|o| format!("{{{}}}", o["name"].as_str().unwrap_or(""))).collect();
            facts.push((tr("Outputs").into(), if outs.is_empty() { tr("the whole answer").into() } else { outs.join(", ") }));
            let eh = &cfg["error_handling"];
            facts.push((tr("Time limit").into(), format!("{} s · {}", eh["timeout_s"].as_u64().unwrap_or(30), ui::retry_label(eh["retry_count"].as_u64().unwrap_or(0) as u32, eh["retry_delay_s"].as_f64().unwrap_or(1.0)))));
            facts.push((tr("Network").into(), tr("through the agents' network policy (kalangod)").into()));
        } else {
            self.s_request.set_text(&it.name);
            facts.push((tr("Level").into(), ui::level_label(&it.level).into()));
            facts.push((tr("Changes").into(), if it.row["read_only"].as_bool().unwrap_or(false) { tr("only reads").into() } else { tr("can change things").into() }));
        }
        for (k, v) in facts {
            let r = caixa(Orientation::Horizontal, 12, "fx-ficha-linha");
            r.append(&ui::left(&k, "fx-ficha-nome"));
            let vl = ui::wrapped(&v, "fx-ficha-valor");
            vl.set_xalign(1.0);
            vl.set_justify(gtk::Justification::Right);
            r.append(&vl);
            self.s_facts.append(&r);
        }
        ui::clear(&self.s_users);
        let (names, _) = users(&self.agents.borrow(), &it.name);
        if names.is_empty() {
            self.s_users.insert(&ui::chip(tr("no agent yet"), ""), -1);
        }
        for n in names {
            let c = ui::chip(&n, "acento");
            c.add_css_class("forte");
            self.s_users.insert(&c, -1);
        }
        self.s_buttons.set_visible(custom);
        self.s_test.set_sensitive(custom);
        self.s_dup.set_sensitive(custom);
        self.s_edit.set_sensitive(custom);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_and_users() {
        let list = json!({
            "system": [{"name": "arquivos.listar", "level": "observar"}, {"name": "correio.enviar", "level": "usuario"}, {"name": "arquivos.restaurar", "level": "usuario"}],
            "custom": [{"id": "1", "name": "clima.previsao", "kind": "http", "config": {"description": "previsão"}, "permission": "free"}]
        });
        let v = classify(&list, &["correio".into()]);
        assert_eq!(v.iter().map(|i| (i.name.as_str(), i.origin)).collect::<Vec<_>>(), vec![("arquivos.listar", "system"), ("correio.enviar", "connector"), ("clima.previsao", "custom")]);
        let agents = vec![
            json!({"name": "A", "config": {"tools": [{"name": "arquivos.listar", "permission": "free"}]}}),
            json!({"name": "B", "config": {"tools": [{"name": "arquivos.listar", "permission": "free"}, {"name": "x", "permission": "ask"}]}}),
        ];
        assert_eq!(users(&agents, "arquivos.listar"), (vec!["A".to_string(), "B".into()], Some("free".into())));
        assert_eq!(users(&agents, "nada"), (vec![], None));
        assert_eq!(users_label(&["A".into(), "B".into()]), trf("{n} agents", &[("n", "2".into())]));
    }
}
