//! Agents center › Agentes (designs "Agentes · biblioteca", "Itens criados pelo agente", "Galeria de modelos",
//! "Ferramentas" and "Janelas estreitas"): the page's tabs — Em execução (the tasks running now, central.rs), Meus
//! agentes (the library of kalango-fluxos: assistants, teams, flows and remote agents, with the chosen one's facts,
//! tools and last runs on the right), Prontos para usar (central_agentes_modelos.rs) and Ferramentas
//! (fluxos_ferramentas.rs). Everything is read and changed through the service (fluxos.rs).
//!
//! Items an agent made (`created_by` = "agent:<id>") have the gold border and the "criado pelo agente" mark, and a
//! banner sums them up with "Ver na Linha do tempo" and "Desfazer" (which deletes them from the service).
//!
//! Narrow windows: the cards take the columns that fit; below 1000 px the side panel becomes a page of its own.

use crate::fluxos;
use crate::fluxos_ui::{self as ui, Segmented, Tabs};
use crate::superficies::{caixa, estrela, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// Where a page of the platform sends the person.
// (the editors that read the fields come with their own pages)
#[allow(dead_code)]
pub enum Go {
    /// a page of the center by its id ("novo-agente", "linha", "agentes"…)
    Page(&'static str),
    /// the agent editor; `new` when it was never saved; `chat` opens it with "Testar agora" ready
    Agent { agent: Value, new: bool, chat: bool },
    /// the HTTP tool editor (None = a new tool; Some = its row from tools.list); `test` with the test panel ready
    Tool { row: Option<Value>, test: bool },
    ImportOpenApi,
    /// the agents center's chat with this agent chosen
    Chat(Value),
    /// runs this flow now and shows it live
    RunFlow(String),
}

pub type Nav = Rc<dyn Fn(Go)>;

/// Below this window width the side panel is a page of its own.
const NARROW: i32 = 1000;

pub(crate) fn now() -> i64 {
    glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0)
}

// ---------------------------------------------------------------- facts of an agent (pure)

/// How many of an assistant's tools are the system's, a connector's and the person's own (HTTP/OpenAPI).
pub fn tool_counts(names: &[&str], connectors: &[String], custom: &[String]) -> (usize, usize, usize) {
    let mut c = (0, 0, 0);
    for n in names {
        if custom.iter().any(|x| x == n) {
            c.2 += 1;
        } else if connectors.iter().any(|x| x == ui::tool_prefix(n)) {
            c.1 += 1;
        } else {
            c.0 += 1;
        }
    }
    c
}

pub fn tools_summary((sys, con, own): (usize, usize, usize)) -> String {
    let mut parts = Vec::new();
    if sys > 0 {
        parts.push(trf("{n} from the system", &[("n", sys.to_string())]));
    }
    if con > 0 {
        parts.push(if con == 1 { tr("1 connector").to_string() } else { trf("{n} connectors", &[("n", con.to_string())]) });
    }
    if own > 0 {
        parts.push(if own == 1 { tr("1 of yours").to_string() } else { trf("{n} of yours", &[("n", own.to_string())]) });
    }
    if parts.is_empty() { tr("none").to_string() } else { parts.join(" · ") }
}

/// The rows of the "ficha" for an agent: (name, value). `names` gives other agents' and flows' names.
pub fn facts(agent: &Value, names: &dyn Fn(&str) -> String, connectors: &[String], custom: &[String]) -> Vec<(String, String)> {
    let c = &agent["config"];
    let mut v: Vec<(String, String)> = Vec::new();
    match ui::kind_of(agent) {
        "assistant" => {
            let model = if c["model_mode"].as_str().unwrap_or("auto") == "auto" { tr("automatic by input · local").to_string() } else { ui::model_label(&c["model"]) };
            v.push((tr("Model").into(), model));
            let fb: Vec<String> = c["fallback"].as_array().into_iter().flatten().map(ui::model_label).collect();
            v.push((tr("Fallback").into(), if fb.is_empty() { tr("none (local only)").to_string() } else { fb.join(", ") }));
            let tools: Vec<&str> = c["tools"].as_array().into_iter().flatten().filter_map(|t| t["name"].as_str()).collect();
            v.push((tr("Tools").into(), tools_summary(tool_counts(&tools, connectors, custom))));
            let m = &c["memory"];
            let conv = m["conversation"].as_bool().unwrap_or(true);
            let long = m["long_term"].as_bool().unwrap_or(false);
            v.push((tr("Memory").into(), match (conv, long) {
                (true, true) => tr("this conversation + long term").to_string(),
                (true, false) => tr("this conversation").to_string(),
                (false, true) => tr("long term").to_string(),
                _ => tr("none").to_string(),
            }));
            let ask = c["tools"].as_array().into_iter().flatten().filter(|t| t["permission"].as_str().unwrap_or("ask") == "ask").count();
            v.push((tr("Autonomy").into(), if tools.is_empty() { "—".to_string() } else if ask == 0 { tr("never asks").to_string() } else if ask == tools.len() { tr("always asks").to_string() } else { trf("asks before {n} tools", &[("n", ask.to_string())]) }));
            let mut out = if c["reply_with_voice"].as_bool().unwrap_or(false) { tr("text and voice").to_string() } else { tr("text").to_string() };
            if let Some(k) = c["output_key"].as_str().filter(|k| !k.is_empty()) {
                out = format!("{out} · {{{k}}}");
            }
            v.push((tr("Output").into(), out));
        }
        k @ ("sequential" | "parallel" | "loop") => {
            v.push((tr("How it works").into(), team_mode_label(k).to_string()));
            let subs: Vec<String> = c["sub_agents"].as_array().into_iter().flatten().filter_map(|s| s.as_str()).map(names).collect();
            v.push((tr("Sub-agents").into(), if subs.is_empty() { tr("none").to_string() } else { subs.join(" → ") }));
            if k == "loop" {
                v.push((tr("Repeats").into(), trf("up to {n} times", &[("n", c["max_iterations"].as_u64().unwrap_or(5).to_string())])));
            }
            if let Some(o) = c["output_key"].as_str().filter(|k| !k.is_empty()) {
                v.push((tr("Output").into(), format!("{{{o}}}")));
            }
        }
        "remote" => {
            v.push((tr("Address").into(), c["card_url"].as_str().unwrap_or("").to_string()));
            v.push((tr("Key").into(), c["key_secret"].as_str().filter(|k| !k.is_empty()).map(|k| format!("{{cofre:{k}}}")).unwrap_or_else(|| tr("none").to_string())));
            v.push((tr("Time limit").into(), format!("{} s", c["timeout_s"].as_u64().unwrap_or(300))));
        }
        _ => {
            let id = c["flow_id"].as_str().unwrap_or("");
            v.push((tr("Flow").into(), names(id)));
        }
    }
    v
}

pub fn team_mode_label(kind: &str) -> &'static str {
    match kind {
        "parallel" => tr("All at the same time"),
        "loop" => tr("Repeat until done"),
        "assistant" => tr("A coordinator decides"),
        _ => tr("One after the other"),
    }
}

// ---------------------------------------------------------------- the page

pub struct AgentsPage {
    pub raiz: gtk::Overlay,
    nav: RefCell<Option<Nav>>,
    tabs: Rc<Tabs>,
    tab: RefCell<String>,
    bodies: gtk::Stack,
    sides: gtk::Stack,
    row: gtk::Box,
    /// list or the side panel as a page (narrow windows)
    main: gtk::Stack,
    holder: gtk::Box,
    narrow: Cell<bool>,
    /// the room the window gives the page (Central's measure)
    room: Cell<i32>,
    import_btn: gtk::Button,
    new_btn: gtk::Button,
    header_error: gtk::Label,
    pub library: Rc<Library>,
    pub gallery: Rc<crate::central_agentes_modelos::Gallery>,
    pub tools: Rc<crate::fluxos_ferramentas::ToolsTab>,
}

impl AgentsPage {
    /// `running` and `running_side` are the "Em execução" body and steps panel (central.rs).
    pub fn new(window_width: i32, running: &gtk::Box, running_side: &gtk::Box) -> Rc<AgentsPage> {
        let pw = ui::panel_width(window_width);
        let raiz = gtk::Overlay::new();
        let row = caixa(Orientation::Horizontal, 0, "");
        row.set_vexpand(true);
        let main = gtk::Stack::new();
        main.set_hexpand(true);
        main.set_hhomogeneous(false);
        main.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        main.set_transition_duration(180);
        let lista = caixa(Orientation::Vertical, 0, "fx-pagina");
        lista.set_hexpand(true);
        // header: title, what it is, the tab's buttons
        let top = caixa(Orientation::Horizontal, 12, "fx-cabecalho");
        let t = caixa(Orientation::Vertical, 4, "");
        t.set_hexpand(true);
        t.append(&ui::left(tr("Agents"), "central-titulo"));
        t.append(&ui::wrapped(tr("Assistants, teams and flows you created. All run on this computer."), "central-sub"));
        top.append(&t);
        let import_btn = ui::button(Some("baixar"), tr("Import"), false);
        import_btn.set_valign(Align::Start);
        top.append(&import_btn);
        let new_btn = ui::button(Some("mais"), tr("New agent"), true);
        new_btn.set_valign(Align::Start);
        top.append(&new_btn);
        lista.append(&top);
        let header_error = ui::error_label();
        header_error.set_margin_start(24);
        header_error.set_margin_end(24);
        lista.append(&header_error);
        let bodies = gtk::Stack::new();
        bodies.set_vexpand(true);
        bodies.set_hhomogeneous(false);
        bodies.set_transition_type(gtk::StackTransitionType::Crossfade);
        bodies.set_transition_duration(120);
        let sides = gtk::Stack::new();
        sides.set_hhomogeneous(false);
        sides.set_transition_type(gtk::StackTransitionType::Crossfade);
        sides.set_transition_duration(120);
        let library = Library::new(pw);
        let gallery = crate::central_agentes_modelos::Gallery::new(pw);
        let tools = crate::fluxos_ferramentas::ToolsTab::new(pw);
        running.add_css_class("fx-execucao");
        bodies.add_named(running, Some("execucao"));
        bodies.add_named(&library.body, Some("meus"));
        bodies.add_named(&gallery.body, Some("prontos"));
        bodies.add_named(&tools.body, Some("ferramentas"));
        sides.add_named(running_side, Some("execucao"));
        sides.add_named(&library.side, Some("meus"));
        sides.add_named(&gallery.side, Some("prontos"));
        sides.add_named(&tools.side, Some("ferramentas"));
        let page: Rc<RefCell<Weak<AgentsPage>>> = Rc::default();
        let p2 = page.clone();
        let tabs = Tabs::new(&[("execucao", tr("In progress")), ("meus", tr("My agents")), ("prontos", tr("Ready to use")), ("ferramentas", tr("Tools"))], move |id| {
            if let Some(p) = p2.borrow().upgrade() {
                p.show_tab(id);
            }
        });
        let tabs_row = caixa(Orientation::Vertical, 0, "fx-abas-linha");
        tabs_row.append(&tabs.root);
        lista.append(&tabs_row);
        lista.append(&bodies);
        main.add_named(&lista, Some("lista"));
        // the side panel as a page: back, then the panel
        let detail = caixa(Orientation::Vertical, 0, "fx-detalhe");
        let back = ui::button(Some("seta-esquerda"), tr("Back"), false);
        back.add_css_class("plano");
        back.set_halign(Align::Start);
        back.set_margin_start(16);
        back.set_margin_top(12);
        detail.append(&back);
        let holder = caixa(Orientation::Vertical, 0, "");
        holder.set_vexpand(true);
        detail.append(&holder);
        main.add_named(&detail, Some("detalhe"));
        row.append(&main);
        row.append(&sides);
        raiz.set_child(Some(&row));
        // what measures the page's width (the window's band)
        let meter = gtk::DrawingArea::new();
        meter.set_can_target(false);
        meter.set_hexpand(true);
        meter.set_vexpand(true);
        raiz.add_overlay(&meter);
        raiz.set_measure_overlay(&meter, false);

        let p = Rc::new(AgentsPage {
            raiz, nav: RefCell::default(), tabs, tab: RefCell::new(String::new()), bodies, sides, row, main, holder, narrow: Cell::new(false), room: Cell::new(i32::MAX), import_btn: import_btn.clone(), new_btn: new_btn.clone(),
            header_error, library, gallery, tools,
        });
        *page.borrow_mut() = Rc::downgrade(&p);
        let w = Rc::downgrade(&p);
        meter.connect_resize(move |_, width, _| {
            if let Some(p) = w.upgrade() {
                // (the page may be wider than the window's room — clipped while the window narrows — so the room
                // the window gives counts too)
                let width = width.min(p.room.get());
                glib::idle_add_local_once(move || p.set_width(width));
            }
        });
        let w = Rc::downgrade(&p);
        back.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.main.set_visible_child_name("lista");
            }
        });
        // a card chosen on a narrow window opens its panel as a page
        for sel in [&p.library.on_pick, &p.gallery.on_pick, &p.tools.on_pick] {
            let w = Rc::downgrade(&p);
            *sel.borrow_mut() = Some(Rc::new(move || {
                if let Some(p) = w.upgrade().filter(|p| p.narrow.get()) {
                    p.main.set_visible_child_name("detalhe");
                }
            }));
        }
        let w = Rc::downgrade(&p);
        new_btn.connect_clicked(move |_| {
            let Some(p) = w.upgrade() else { return };
            if *p.tab.borrow() == "ferramentas" {
                p.go(Go::Tool { row: None, test: false });
            } else {
                // the form; "Criar agente (por descrição)" stays reachable from the conversations
                p.go(Go::Agent { agent: Value::Null, new: true, chat: false });
            }
        });
        let w = Rc::downgrade(&p);
        import_btn.connect_clicked(move |b| {
            let Some(p) = w.upgrade() else { return };
            if *p.tab.borrow() == "ferramentas" {
                p.go(Go::ImportOpenApi);
            } else {
                p.import_agents(b);
            }
        });
        p.show_tab("meus");
        p
    }

    pub fn set_nav(&self, nav: Nav) {
        *self.library.nav.borrow_mut() = Some(nav.clone());
        *self.gallery.nav.borrow_mut() = Some(nav.clone());
        *self.tools.nav.borrow_mut() = Some(nav.clone());
        *self.nav.borrow_mut() = Some(nav);
    }

    fn go(&self, g: Go) {
        if let Some(n) = self.nav.borrow().clone() {
            n(g);
        }
    }

    pub fn show_tab(self: &Rc<Self>, id: &str) {
        *self.tab.borrow_mut() = id.to_string();
        self.tabs.show(id);
        self.bodies.set_visible_child_name(id);
        self.sides.set_visible_child_name(id);
        self.main.set_visible_child_name("lista");
        ui::show_error(&self.header_error, "");
        let tools = id == "ferramentas";
        ui::set_button_text(&self.import_btn, if tools { tr("Import OpenAPI") } else { tr("Import") });
        ui::set_button_text(&self.new_btn, if tools { tr("New tool") } else { tr("New agent") });
        self.import_btn.set_visible(id != "execucao");
        match id {
            "meus" => self.library.reload(),
            "prontos" => self.gallery.render(),
            "ferramentas" => self.tools.reload(),
            _ => {}
        }
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug central: agentes aba {id}");
        }
    }

    /// The room the window gives the page: below the wide layout's minimum the page folds (frame 12).
    pub fn set_room(self: &Rc<Self>, width: i32) {
        self.room.set(width);
        self.set_width(width);
    }

    /// Reads the data again (coming back from the editor).
    pub fn reload(self: &Rc<Self>) {
        let t = self.tab.borrow().clone();
        self.show_tab(&t);
    }

    fn set_width(&self, width: i32) {
        let narrow = width < NARROW;
        self.library.set_width(width);
        self.gallery.set_width(width);
        if narrow == self.narrow.get() {
            return;
        }
        self.narrow.set(narrow);
        if narrow {
            self.row.remove(&self.sides);
            self.holder.append(&self.sides);
            self.sides.set_hexpand(true);
        } else {
            self.holder.remove(&self.sides);
            self.row.append(&self.sides);
            self.sides.set_hexpand(false);
            self.main.set_visible_child_name("lista");
        }
        for p in [&self.library.side, &self.gallery.side, &self.tools.side] {
            p.set_size_request(if narrow { -1 } else { ui::panel_width(width + 64) }, -1);
            if narrow { p.add_css_class("estreito") } else { p.remove_css_class("estreito") }
        }
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug central: agentes estreita {narrow} ({width} px)");
        }
    }

    /// "Importar": agents from a JSON file (one agent or a list), each saved with a fresh id.
    fn import_agents(self: &Rc<Self>, b: &gtk::Button) {
        let f = gtk::FileFilter::new();
        f.set_name(Some(tr("Agents (JSON)")));
        f.add_pattern("*.json");
        let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&f);
        let d = gtk::FileDialog::new();
        d.set_title(tr("Import agents"));
        d.set_filters(Some(&filters));
        let w = Rc::downgrade(self);
        d.open(b.root().and_downcast_ref::<gtk::Window>(), gtk::gio::Cancellable::NONE, move |r| {
            let (Some(p), Ok(file)) = (w.upgrade(), r) else { return };
            let Some(path) = file.path() else { return };
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let agents: Vec<Value> = match serde_json::from_str::<Value>(&text) {
                Ok(Value::Array(a)) => a,
                Ok(v @ Value::Object(_)) => vec![v.get("agent").cloned().unwrap_or(v)],
                _ => {
                    ui::show_error(&p.header_error, tr("The file is not an agent in JSON."));
                    return;
                }
            };
            for mut a in agents {
                a["id"] = json!(ui::new_id("agente"));
                let w = Rc::downgrade(&p);
                fluxos::call("agents.save", json!({"agent": a}), move |r| {
                    let Some(p) = w.upgrade() else { return };
                    match r {
                        Ok(_) => p.library.reload(),
                        Err(e) => ui::show_error(&p.header_error, &trf("Could not import: {error}", &[("error", e)])),
                    }
                });
            }
        });
    }
}

// ---------------------------------------------------------------- Meus agentes

/// One item of the library: an agent, or a flow no agent wraps.
#[derive(Clone)]
struct Item {
    id: String,
    agent: Value,
    created_by: String,
    /// the flow's id (flow agents and bare flows)
    flow_id: Option<String>,
    bare_flow: bool,
}

pub struct Library {
    pub body: gtk::Box,
    pub side: gtk::Box,
    pub nav: RefCell<Option<Nav>>,
    pub on_pick: RefCell<Option<Rc<dyn Fn()>>>,
    banner: gtk::Box,
    banner_title: gtk::Label,
    banner_sub: gtk::Label,
    undo: gtk::Button,
    undo_armed: Cell<bool>,
    query: gtk::Entry,
    filter: RefCell<String>,
    seg: Segmented,
    /// the kinds as one choice, in a narrow window
    kinds: gtk::DropDown,
    search_box: gtk::Box,
    order: gtk::DropDown,
    by: gtk::DropDown,
    grid: gtk::FlowBox,
    empty: gtk::Label,
    items: RefCell<Vec<Item>>,
    flows: RefCell<Vec<Value>>,
    runs: RefCell<Vec<Value>>,
    custom_tools: RefCell<Vec<Value>>,
    system_tools: RefCell<Vec<Value>>,
    chosen: RefCell<String>,
    enabled: RefCell<std::collections::HashMap<String, bool>>,
    loading: Cell<u32>,
    // the side panel's parts
    s_tile: gtk::Box,
    s_name: gtk::Label,
    s_sub: gtk::Label,
    s_switch: gtk::Switch,
    s_desc: gtk::Label,
    s_facts: gtk::Box,
    s_uses_title: gtk::Label,
    s_uses: gtk::FlowBox,
    s_runs: gtk::Box,
    s_content: gtk::Box,
    s_empty: gtk::Label,
    s_error: gtk::Label,
    s_buttons: gtk::Box,
    filling: Cell<bool>,
}

impl Library {
    fn new(panel_width: i32) -> Rc<Library> {
        let body = caixa(Orientation::Vertical, 0, "fx-corpo");
        body.set_hexpand(true);
        // the summary of what an agent made
        let banner = caixa(Orientation::Horizontal, 12, "fx-aviso-agente");
        let star = estrela(13, "#E5C98C");
        star.set_valign(Align::Center);
        banner.append(&star);
        let bt = caixa(Orientation::Vertical, 2, "");
        bt.set_hexpand(true);
        bt.set_valign(Align::Center);
        let banner_title = ui::wrapped("", "fx-aviso-titulo");
        bt.append(&banner_title);
        let banner_sub = ui::wrapped("", "fx-aviso-sub");
        bt.append(&banner_sub);
        banner.append(&bt);
        let see = ui::button(Some("relogio"), tr("See in the Timeline"), false);
        see.add_css_class("plano");
        see.set_valign(Align::Center);
        banner.append(&see);
        let undo = ui::button(None, tr("Undo"), false);
        undo.set_valign(Align::Center);
        banner.append(&undo);
        banner.set_visible(false);
        body.append(&banner);
        // search, kinds, order
        let bar = caixa(Orientation::Horizontal, 14, "fx-filtros");
        let (sb, query) = ui::search(tr("Search agents…"));
        sb.set_size_request(260, -1);
        sb.set_hexpand(false);
        bar.append(&sb);
        let lib: Rc<RefCell<Weak<Library>>> = Rc::default();
        let l2 = lib.clone();
        let seg = Segmented::new(
            &[("all".into(), tr("All").into()), ("assistants".into(), tr("Assistants").into()), ("teams".into(), tr("Teams").into()), ("flows".into(), tr("Flows").into()), ("remotes".into(), tr("Remotes").into())],
            "all",
            move |id| {
                if let Some(l) = l2.borrow().upgrade() {
                    *l.filter.borrow_mut() = id.to_string();
                    l.render();
                }
            },
        );
        seg.root.add_css_class("fx-seg-filtros");
        bar.append(&seg.root);
        // (a narrow window, frame 12: the kinds fold into one choice, "Todos ▾")
        let kind_ids = ["all", "assistants", "teams", "flows", "remotes"];
        let kinds = ui::dropdown(&[tr("All kinds").to_string(), tr("Assistants").to_string(), tr("Teams").to_string(), tr("Flows").to_string(), tr("Remotes").to_string()], 0);
        kinds.add_css_class("fx-ordem");
        kinds.set_hexpand(false);
        kinds.set_visible(false);
        let l3 = lib.clone();
        kinds.connect_selected_notify(move |d| {
            let Some(l) = l3.borrow().upgrade() else { return };
            let id = kind_ids.get(d.selected() as usize).copied().unwrap_or("all");
            if *l.filter.borrow() != id {
                *l.filter.borrow_mut() = id.to_string();
                l.seg.set(id);
                l.render();
            }
        });
        bar.append(&kinds);
        let sp = caixa(Orientation::Horizontal, 0, "");
        sp.set_hexpand(true);
        bar.append(&sp);
        let order = ui::dropdown(&[tr("Order: changed recently").to_string(), tr("Order: name").to_string()], 0);
        order.add_css_class("fx-ordem");
        order.set_hexpand(false);
        bar.append(&order);
        let by = ui::dropdown(&[tr("Created by: everyone").to_string(), tr("Created by: you").to_string(), tr("Created by: the agent").to_string()], 0);
        by.add_css_class("fx-ordem");
        by.set_hexpand(false);
        by.set_visible(false);
        bar.append(&by);
        body.append(&bar);
        let grid = gtk::FlowBox::new();
        grid.add_css_class("fx-grade");
        grid.set_selection_mode(gtk::SelectionMode::None);
        grid.set_homogeneous(true);
        grid.set_column_spacing(13);
        grid.set_row_spacing(13);
        grid.set_min_children_per_line(1);
        grid.set_max_children_per_line(4);
        grid.set_valign(Align::Start);
        let empty = ui::wrapped("", "central-sub");
        empty.set_visible(false);
        let col = caixa(Orientation::Vertical, 12, "");
        col.append(&empty);
        col.append(&grid);
        body.append(&ui::scroll(&col));

        // the chosen agent
        let side = ui::side_panel(panel_width);
        side.append(&ui::panel_title("spark", tr("SELECTED AGENT")));
        let s_content = caixa(Orientation::Vertical, 0, "");
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
        let s_switch = ui::switch();
        s_switch.set_valign(Align::Start);
        s_switch.set_margin_top(2);
        s_switch.set_tooltip_text(Some(tr("Turn this agent on or off")));
        head.append(&s_switch);
        s_content.append(&head);
        let s_desc = ui::wrapped("", "fx-descricao");
        s_desc.set_margin_top(14);
        s_content.append(&s_desc);
        let s_facts = caixa(Orientation::Vertical, 0, "fx-ficha");
        s_facts.set_margin_top(14);
        s_content.append(&s_facts);
        let s_uses_title = ui::section(tr("CAN USE"));
        s_uses_title.set_margin_top(20);
        s_content.append(&s_uses_title);
        let s_uses = gtk::FlowBox::new();
        s_uses.add_css_class("fx-chips");
        s_uses.set_selection_mode(gtk::SelectionMode::None);
        s_uses.set_column_spacing(6);
        s_uses.set_row_spacing(6);
        s_uses.set_max_children_per_line(30);
        s_uses.set_margin_top(10);
        s_content.append(&s_uses);
        let rt = ui::section(tr("LAST RUNS"));
        rt.set_margin_top(20);
        s_content.append(&rt);
        let s_runs = caixa(Orientation::Vertical, 0, "fx-execucoes");
        s_runs.set_margin_top(10);
        s_content.append(&s_runs);
        let s_empty = ui::wrapped(tr("Choose an agent to see what it does."), "central-sub");
        s_empty.set_margin_top(16);
        let inner = caixa(Orientation::Vertical, 0, "");
        inner.append(&s_content);
        inner.append(&s_empty);
        side.append(&ui::scroll(&inner));
        let s_error = ui::error_label();
        side.append(&s_error);
        let s_buttons = caixa(Orientation::Horizontal, 8, "fx-botoes-painel");
        s_buttons.set_homogeneous(true);
        let talk = ui::button(Some("enviar"), tr("Chat"), false);
        let dup = ui::button(Some("compartilhar"), tr("Make a copy"), false);
        let edit = ui::button(Some("ajustes"), tr("Edit agent"), true);
        s_buttons.append(&talk);
        s_buttons.append(&dup);
        s_buttons.append(&edit);
        side.append(&s_buttons);

        let l = Rc::new(Library {
            body, side, nav: RefCell::default(), on_pick: RefCell::default(), banner, banner_title, banner_sub, undo: undo.clone(), undo_armed: Cell::new(false), query: query.clone(),
            filter: RefCell::new("all".into()), seg, kinds, search_box: sb.clone(), order: order.clone(), by: by.clone(), grid, empty, items: RefCell::default(), flows: RefCell::default(), runs: RefCell::default(),
            custom_tools: RefCell::default(), system_tools: RefCell::default(), chosen: RefCell::default(), enabled: RefCell::default(), loading: Cell::new(0), s_tile, s_name, s_sub,
            s_switch: s_switch.clone(), s_desc, s_facts, s_uses_title, s_uses, s_runs, s_content, s_empty, s_error, s_buttons, filling: Cell::new(false),
        });
        *lib.borrow_mut() = Rc::downgrade(&l);
        let w = Rc::downgrade(&l);
        query.connect_changed(move |_| {
            if let Some(l) = w.upgrade() {
                l.render();
            }
        });
        for d in [&order, &by] {
            let w = Rc::downgrade(&l);
            d.connect_selected_notify(move |_| {
                if let Some(l) = w.upgrade() {
                    l.render();
                }
            });
        }
        let w = Rc::downgrade(&l);
        see.connect_clicked(move |_| {
            if let Some(l) = w.upgrade() {
                l.go(Go::Page("linha"));
            }
        });
        let w = Rc::downgrade(&l);
        undo.connect_clicked(move |_| {
            if let Some(l) = w.upgrade() {
                l.undo_made_by_agent();
            }
        });
        let w = Rc::downgrade(&l);
        s_switch.connect_active_notify(move |s| {
            let Some(l) = w.upgrade().filter(|l| !l.filling.get()) else { return };
            l.set_enabled(s.is_active());
        });
        let w = Rc::downgrade(&l);
        talk.connect_clicked(move |_| {
            if let Some((l, it)) = w.upgrade().and_then(|l| l.current().map(|i| (l, i))) {
                match it.flow_id {
                    Some(f) => l.go(Go::RunFlow(f)),
                    None => l.go(Go::Chat(it.agent)),
                }
            }
        });
        let w = Rc::downgrade(&l);
        dup.connect_clicked(move |_| {
            if let Some(l) = w.upgrade() {
                l.duplicate();
            }
        });
        let w = Rc::downgrade(&l);
        edit.connect_clicked(move |_| {
            if let Some((l, it)) = w.upgrade().and_then(|l| l.current().map(|i| (l, i))) {
                l.go(Go::Agent { agent: it.agent, new: false, chat: false });
            }
        });
        l
    }

    fn go(&self, g: Go) {
        if let Some(n) = self.nav.borrow().clone() {
            n(g);
        }
    }

    fn current(&self) -> Option<Item> {
        let c = self.chosen.borrow().clone();
        self.items.borrow().iter().find(|i| i.id == c).cloned()
    }

    fn set_width(&self, width: i32) {
        // the window's band (frame 12): narrow, the kinds are one choice and the order goes, so the row fits
        let narrow = width < NARROW;
        self.seg.root.set_visible(!narrow);
        self.order.set_visible(!narrow);
        self.kinds.set_visible(narrow);
        self.search_box.set_hexpand(narrow);
        self.search_box.set_size_request(if narrow { 160 } else { 260 }, -1);
        let ids = ["all", "assistants", "teams", "flows", "remotes"];
        if let Some(i) = ids.iter().position(|k| *k == self.filter.borrow().as_str()) {
            self.kinds.set_selected(i as u32);
        }
    }

    /// Everything again from the service: agents, flows, runs and tools.
    pub fn reload(self: &Rc<Self>) {
        self.loading.set(4);
        let w = Rc::downgrade(self);
        fluxos::call("agents.list", json!({}), move |r| {
            let Some(l) = w.upgrade() else { return };
            match r {
                Ok(v) => {
                    let items = v.as_array().into_iter().flatten().map(|x| {
                        let a = x["agent"].clone();
                        let flow_id = (ui::kind_of(&a) == "flow").then(|| a["config"]["flow_id"].as_str().unwrap_or("").to_string());
                        Item { id: a["id"].as_str().unwrap_or("").to_string(), agent: a, created_by: x["created_by"].as_str().unwrap_or("person").to_string(), flow_id, bare_flow: false }
                    }).collect();
                    *l.items.borrow_mut() = items;
                    ui::show_error(&l.s_error, "");
                }
                Err(e) => {
                    l.items.borrow_mut().clear();
                    l.empty.set_text(&trf("The agents service does not answer ({error}).", &[("error", e)]));
                }
            }
            l.loaded();
        });
        let w = Rc::downgrade(self);
        fluxos::call("flows.list", json!({}), move |r| {
            let Some(l) = w.upgrade() else { return };
            *l.flows.borrow_mut() = r.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
            l.loaded();
        });
        let w = Rc::downgrade(self);
        fluxos::call("runs.list", json!({"limit": 200}), move |r| {
            let Some(l) = w.upgrade() else { return };
            *l.runs.borrow_mut() = r.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
            l.loaded();
        });
        let w = Rc::downgrade(self);
        fluxos::call("tools.list", json!({}), move |r| {
            let Some(l) = w.upgrade() else { return };
            let v = r.unwrap_or(json!({}));
            *l.custom_tools.borrow_mut() = v["custom"].as_array().cloned().unwrap_or_default();
            *l.system_tools.borrow_mut() = v["system"].as_array().cloned().unwrap_or_default();
            l.loaded();
        });
    }

    fn loaded(self: &Rc<Self>) {
        self.loading.set(self.loading.get().saturating_sub(1));
        if self.loading.get() > 0 {
            return;
        }
        // flows no agent wraps are items of their own
        let wrapped: Vec<String> = self.items.borrow().iter().filter_map(|i| i.flow_id.clone()).collect();
        let bare: Vec<Item> = self.flows.borrow().iter().filter(|f| !wrapped.iter().any(|w| f["id"].as_str() == Some(w))).map(|f| {
            let id = f["id"].as_str().unwrap_or("").to_string();
            Item {
                id: format!("fluxo:{id}"),
                agent: json!({"id": id, "name": f["name"], "description": "", "kind": "flow", "config": {"flow_id": id}}),
                created_by: f["created_by"].as_str().unwrap_or("person").to_string(),
                flow_id: Some(id),
                bare_flow: true,
            }
        }).collect();
        self.items.borrow_mut().extend(bare);
        // flows know whether they are on
        for f in self.flows.borrow().iter() {
            if let (Some(id), Some(on)) = (f["id"].as_str(), f["enabled"].as_bool()) {
                for it in self.items.borrow().iter().filter(|i| i.flow_id.as_deref() == Some(id)) {
                    self.enabled.borrow_mut().entry(it.id.clone()).or_insert(on);
                }
            }
        }
        let made = self.items.borrow().iter().any(|i| ui::made_by_agent(&i.created_by));
        self.by.set_visible(made);
        self.order.set_visible(!made);
        if self.current().is_none() {
            let first = self.items.borrow().first().map(|i| i.id.clone()).unwrap_or_default();
            *self.chosen.borrow_mut() = first;
        }
        self.undo_armed.set(false);
        self.render();
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug central: biblioteca {} itens", self.items.borrow().len());
        }
    }

    fn name_of(&self, id: &str) -> String {
        if let Some(i) = self.items.borrow().iter().find(|i| i.id == id) {
            return i.agent["name"].as_str().unwrap_or(id).to_string();
        }
        if let Some(f) = self.flows.borrow().iter().find(|f| f["id"].as_str() == Some(id)) {
            return f["name"].as_str().unwrap_or(id).to_string();
        }
        id.to_string()
    }

    fn last_run(&self, it: &Item) -> Option<Value> {
        self.runs.borrow().iter().find(|r| run_belongs(r, it)).cloned()
    }

    fn render(self: &Rc<Self>) {
        let items = self.items.borrow().clone();
        // the counts on the filter
        let count = |f: &str| items.iter().filter(|i| ui::kind_in_filter(f, ui::kind_of(&i.agent))).count();
        self.seg.set_label("all", &trf("All {n}", &[("n", items.len().to_string())]));
        let _ = count;
        // the banner: what an agent made
        let made: Vec<&Item> = items.iter().filter(|i| ui::made_by_agent(&i.created_by)).collect();
        let made_tools: Vec<Value> = self.custom_tools.borrow().iter().filter(|t| ui::made_by_agent(t["created_by"].as_str().unwrap_or(""))).cloned().collect();
        let total = made.len() + made_tools.len();
        self.banner.set_visible(total > 0);
        if total > 0 {
            let agents = if made.len() == 1 { tr("1 agent").to_string() } else { trf("{n} agents", &[("n", made.len().to_string())]) };
            let title = if made_tools.is_empty() {
                trf("An agent created {agents}", &[("agents", agents)])
            } else {
                let tools = if made_tools.len() == 1 { tr("1 tool").to_string() } else { trf("{n} tools", &[("n", made_tools.len().to_string())]) };
                trf("An agent created {agents} and {tools}", &[("agents", agents), ("tools", tools)])
            };
            self.banner_title.set_text(&title);
            let names: Vec<String> = made.iter().map(|i| i.agent["name"].as_str().unwrap_or("").to_string()).chain(made_tools.iter().map(|t| t["name"].as_str().unwrap_or("").to_string())).collect();
            self.banner_sub.set_text(&trf("{names}, created with your approval. Undoing deletes them.", &[("names", names.join(", "))]));
            if !self.undo_armed.get() {
                ui::set_button_text(&self.undo, &if total == 2 { tr("Undo both").to_string() } else if total == 1 { tr("Undo").to_string() } else { trf("Undo the {n}", &[("n", total.to_string())]) });
            }
        }
        // the cards
        let q = self.query.text().to_string();
        let filter = self.filter.borrow().clone();
        let by = self.by.selected();
        let mut shown: Vec<Item> = items
            .into_iter()
            .filter(|i| ui::kind_in_filter(&filter, ui::kind_of(&i.agent)))
            .filter(|i| ui::matches_query(&q, &[i.agent["name"].as_str().unwrap_or(""), i.agent["description"].as_str().unwrap_or("")]))
            .filter(|i| !self.by.is_visible() || by == 0 || (by == 2) == ui::made_by_agent(&i.created_by))
            .collect();
        if self.order.is_visible() && self.order.selected() == 1 {
            shown.sort_by_key(|i| ui::fold(i.agent["name"].as_str().unwrap_or("")));
        }
        ui::clear(&self.grid);
        let chosen = self.chosen.borrow().clone();
        for it in &shown {
            let card = self.card(it, it.id == chosen);
            card.set_size_request(250, -1);
            self.grid.insert(&card, -1);
        }
        let none = shown.is_empty();
        self.empty.set_visible(none);
        if none && self.empty.text().is_empty() || none && !q.is_empty() {
            self.empty.set_text(if self.items.borrow().is_empty() { tr("No agent yet. Create one with “New agent” or start from “Ready to use”.") } else { tr("No agent matches.") });
        }
        self.render_side();
    }

    fn card(self: &Rc<Self>, it: &Item, chosen: bool) -> gtk::Box {
        let a = &it.agent;
        let kind = ui::kind_of(a);
        let c = caixa(Orientation::Vertical, 0, "fx-cartao");
        if chosen {
            c.add_css_class("escolhido");
        }
        let made = ui::made_by_agent(&it.created_by);
        if made {
            c.add_css_class("criado");
        }
        let top = caixa(Orientation::Horizontal, 12, "");
        let tile = crate::agent_picker::icon_tile(a, 32);
        tile.set_valign(Align::Start);
        top.append(&tile);
        let tc = caixa(Orientation::Vertical, 1, "");
        tc.set_hexpand(true);
        tc.append(&ui::cut(a["name"].as_str().unwrap_or(""), "fx-cartao-nome"));
        tc.append(&ui::cut(ui::kind_label(kind), "fx-cartao-tipo"));
        top.append(&tc);
        let more = gtk::MenuButton::new();
        more.add_css_class("fx-mais");
        more.set_child(Some(&crate::conversa::ui_icon("pontos-v", ui::MUTED, 14)));
        more.set_valign(Align::Start);
        let pop = gtk::Popover::new();
        let pc = caixa(Orientation::Vertical, 2, "");
        let mut actions: Vec<(&str, &str)> = vec![("editar", tr("Edit")), ("duplicar", tr("Make a copy")), ("exportar", tr("Export…")), ("apagar", tr("Delete"))];
        if it.bare_flow {
            actions = vec![("apagar", tr("Delete"))];
        }
        // chat with it, or run it (a flow)
        actions.insert(0, if it.flow_id.is_some() { ("executar", tr("Run")) } else { ("conversar", tr("Chat")) });
        for (id, name) in actions {
            let b = gtk::Button::with_label(name);
            b.add_css_class("central-menu-item");
            let (w, id, item, pop2) = (Rc::downgrade(self), id.to_string(), it.clone(), pop.clone());
            b.connect_clicked(move |b| {
                pop2.popdown();
                let Some(l) = w.upgrade() else { return };
                *l.chosen.borrow_mut() = item.id.clone();
                match id.as_str() {
                    "editar" => l.go(Go::Agent { agent: item.agent.clone(), new: false, chat: false }),
                    "conversar" => l.go(Go::Chat(item.agent.clone())),
                    "executar" => l.go(Go::RunFlow(item.flow_id.clone().unwrap_or_default())),
                    "duplicar" => l.duplicate(),
                    "exportar" => export_agent(b, &item.agent),
                    _ => l.delete(&item),
                }
            });
            pc.append(&b);
        }
        pop.set_child(Some(&pc));
        more.set_popover(Some(&pop));
        top.append(&more);
        c.append(&top);
        let desc = a["description"].as_str().filter(|d| !d.is_empty()).map(str::to_string).unwrap_or_else(|| default_description(a));
        let d = ui::wrapped(&desc, "fx-cartao-desc");
        d.set_lines(2);
        d.set_ellipsize(gtk::pango::EllipsizeMode::End);
        d.set_margin_top(12);
        d.set_valign(Align::Start);
        c.append(&d);
        let sp = caixa(Orientation::Vertical, 0, "");
        sp.set_vexpand(true);
        c.append(&sp);
        let chips = caixa(Orientation::Horizontal, 6, "");
        chips.set_margin_top(10);
        if made {
            let m = caixa(Orientation::Horizontal, 4, "fx-chip");
            m.add_css_class("ouro");
            m.add_css_class("forte");
            m.append(&estrela(9, "#E5C98C"));
            m.append(&rotulo(tr("created by the agent"), ""));
            chips.append(&m);
        }
        for t in card_chips(a, it.bare_flow).into_iter().take(if made { 1 } else { 2 }) {
            let ch = ui::chip(&t, "");
            ch.set_ellipsize(gtk::pango::EllipsizeMode::End);
            chips.append(&ch);
        }
        c.append(&chips);
        let st = caixa(Orientation::Horizontal, 6, "");
        st.set_margin_top(10);
        let (text, tone) = match self.last_run(it) {
            Some(r) => {
                let (s, tone) = ui::run_status(r["status"].as_str().unwrap_or(""));
                (format!("{} · {s}", ui::day_and_time(now(), r["created_at"].as_i64().unwrap_or(0))), tone)
            }
            None if made => (tr("created by the agent · has not run yet").to_string(), "teal"),
            None => (tr("never ran").to_string(), "cinza"),
        };
        let dot = caixa(Orientation::Horizontal, 0, "fx-ponto");
        dot.add_css_class(tone);
        dot.set_valign(Align::Center);
        st.append(&dot);
        let sl = ui::cut(&text, "fx-cartao-estado");
        sl.add_css_class(tone);
        st.append(&sl);
        c.append(&st);
        let click = gtk::GestureClick::new();
        let (w, id) = (Rc::downgrade(self), it.id.clone());
        click.connect_released(move |_, n, _, _| {
            let Some(l) = w.upgrade() else { return };
            *l.chosen.borrow_mut() = id.clone();
            if n == 2 {
                if let Some(it) = l.current().filter(|i| !i.bare_flow) {
                    l.go(Go::Agent { agent: it.agent, new: false, chat: false });
                    return;
                }
            }
            l.render();
            let pick = l.on_pick.borrow().clone();
            if let Some(f) = pick {
                f();
            }
        });
        c.add_controller(click);
        crate::central::posicao_debug(&format!("cartao {}", a["name"].as_str().unwrap_or("")), c.upcast_ref());
        c
    }

    fn render_side(self: &Rc<Self>) {
        let Some(it) = self.current() else {
            self.s_content.set_visible(false);
            self.s_buttons.set_visible(false);
            self.s_empty.set_visible(true);
            return;
        };
        self.s_content.set_visible(true);
        self.s_buttons.set_visible(true);
        self.s_empty.set_visible(false);
        let a = &it.agent;
        let kind = ui::kind_of(a);
        ui::clear_box(&self.s_tile);
        self.s_tile.append(&crate::agent_picker::icon_tile(a, 44));
        self.s_name.set_text(a["name"].as_str().unwrap_or(""));
        let who = if ui::made_by_agent(&it.created_by) { tr("created by the agent") } else { tr("created by you") };
        self.s_sub.set_text(&format!("{} · {who}", ui::kind_label(kind)));
        self.filling.set(true);
        self.s_switch.set_active(self.enabled.borrow().get(&it.id).copied().unwrap_or(true));
        self.filling.set(false);
        let desc = a["description"].as_str().filter(|d| !d.is_empty()).map(str::to_string).unwrap_or_else(|| default_description(a));
        self.s_desc.set_text(&desc);
        // the facts
        ui::clear_box(&self.s_facts);
        let connectors: Vec<String> = ui::connectors().into_iter().map(|c| c.0).collect();
        let custom: Vec<String> = self.custom_tools.borrow().iter().filter_map(|t| t["name"].as_str().map(str::to_string)).collect();
        let me = self.clone();
        for (k, v) in facts(a, &move |id| me.name_of(id), &connectors, &custom) {
            let r = caixa(Orientation::Horizontal, 12, "fx-ficha-linha");
            r.append(&ui::left(&k, "fx-ficha-nome"));
            let vl = ui::cut(&v, "fx-ficha-valor");
            vl.set_xalign(1.0);
            r.append(&vl);
            self.s_facts.append(&r);
        }
        // what it can use
        ui::clear(&self.s_uses);
        let c = &a["config"];
        let mut chips: Vec<(String, &'static str)> = Vec::new();
        match kind {
            "assistant" => {
                for t in c["tools"].as_array().into_iter().flatten() {
                    let n = t["name"].as_str().unwrap_or("");
                    let level = self.system_tools.borrow().iter().find(|s| s["name"].as_str() == Some(n)).and_then(|s| s["level"].as_str().map(str::to_string));
                    let tone = if custom.iter().any(|x| x == n) { "acento" } else if connectors.iter().any(|x| x == ui::tool_prefix(n)) { "lilas" } else { level.as_deref().map(ui::level_tone).unwrap_or("teal") };
                    chips.push((n.to_string(), tone));
                }
                for s in c["sub_agents"].as_array().into_iter().flatten() {
                    chips.push((self.name_of(s["agent_id"].as_str().unwrap_or("")), "lilas"));
                }
            }
            "sequential" | "parallel" | "loop" => {
                for s in c["sub_agents"].as_array().into_iter().flatten().filter_map(|s| s.as_str()) {
                    chips.push((self.name_of(s), "lilas"));
                }
            }
            _ => {}
        }
        self.s_uses_title.set_visible(!chips.is_empty());
        self.s_uses.set_visible(!chips.is_empty());
        for (t, tone) in chips {
            let ch = ui::chip(&t, tone);
            ch.add_css_class("forte");
            self.s_uses.insert(&ch, -1);
        }
        // last runs (the service links runs to flows)
        ui::clear_box(&self.s_runs);
        let runs: Vec<Value> = self.runs.borrow().iter().filter(|r| run_belongs(r, &it)).take(5).cloned().collect();
        if runs.is_empty() {
            let l = ui::wrapped(if it.flow_id.is_some() { tr("This flow has not run yet.") } else { tr("Runs show here when the agent runs: in a conversation, a flow or a routine.") }, "central-agente-sub");
            l.set_margin_top(10);
            l.set_margin_bottom(10);
            self.s_runs.append(&l);
        }
        for r in runs {
            let row = caixa(Orientation::Horizontal, 10, "fx-execucao-linha");
            row.append(&ui::left(&ui::day_and_time(now(), r["created_at"].as_i64().unwrap_or(0)), "fx-hora"));
            let what = r["output"].as_str().map(|o| o.lines().next().unwrap_or("").to_string()).filter(|o| !o.is_empty()).or_else(|| r["error_code"].as_str().map(str::to_string)).unwrap_or_default();
            row.append(&ui::cut(&what, "fx-execucao-texto"));
            let (s, tone) = ui::run_status(r["status"].as_str().unwrap_or(""));
            row.append(&ui::chip(s, tone));
            self.s_runs.append(&row);
        }
        // (a flow no agent wraps has nothing to chat with, copy or edit here)
        let mut b = self.s_buttons.first_child();
        while let Some(x) = b {
            x.set_sensitive(!it.bare_flow);
            b = x.next_sibling();
        }
        // the first one chats with it, or runs it (a flow, also one no agent wraps)
        if let Some(talk) = self.s_buttons.first_child().and_downcast::<gtk::Button>() {
            talk.set_sensitive(true);
            ui::set_button_text(&talk, if it.flow_id.is_some() { tr("Run") } else { tr("Chat") });
        }
    }

    fn set_enabled(self: &Rc<Self>, on: bool) {
        let Some(it) = self.current() else { return };
        self.enabled.borrow_mut().insert(it.id.clone(), on);
        let (method, id) = if it.bare_flow { ("flows.enable", it.flow_id.clone().unwrap_or_default()) } else { ("agents.enable", it.id.clone()) };
        let w = Rc::downgrade(self);
        fluxos::call(method, json!({"id": id, "on": on}), move |r| {
            if let (Some(l), Err(e)) = (w.upgrade(), r) {
                ui::show_error(&l.s_error, &e);
            }
        });
        // a flow agent turns its flow on and off too
        if let (false, Some(f)) = (it.bare_flow, it.flow_id) {
            fluxos::call("flows.enable", json!({"id": f, "on": on}), |_| {});
        }
    }

    fn duplicate(self: &Rc<Self>) {
        let Some(it) = self.current().filter(|i| !i.bare_flow) else { return };
        let mut a = it.agent.clone();
        let id = ui::new_id("agente");
        a["id"] = json!(id);
        a["name"] = json!(trf("{name} (copy)", &[("name", a["name"].as_str().unwrap_or("").to_string())]));
        let w = Rc::downgrade(self);
        fluxos::call("agents.save", json!({"agent": a}), move |r| {
            let Some(l) = w.upgrade() else { return };
            match r {
                Ok(_) => {
                    *l.chosen.borrow_mut() = id;
                    l.reload();
                }
                Err(e) => ui::show_error(&l.s_error, &e),
            }
        });
    }

    fn delete(self: &Rc<Self>, it: &Item) {
        let (method, id) = if it.bare_flow { ("flows.delete", it.flow_id.clone().unwrap_or_default()) } else { ("agents.delete", it.id.clone()) };
        let w = Rc::downgrade(self);
        fluxos::call(method, json!({"id": id}), move |r| {
            let Some(l) = w.upgrade() else { return };
            if let Err(e) = r {
                ui::show_error(&l.s_error, &e);
            }
            l.chosen.borrow_mut().clear();
            l.reload();
        });
    }

    /// "Desfazer": deletes what an agent made (a second click confirms).
    fn undo_made_by_agent(self: &Rc<Self>) {
        if !self.undo_armed.get() {
            self.undo_armed.set(true);
            ui::set_button_text(&self.undo, tr("Click again to delete"));
            return;
        }
        let made: Vec<Item> = self.items.borrow().iter().filter(|i| ui::made_by_agent(&i.created_by)).cloned().collect();
        for it in &made {
            let (method, id) = if it.bare_flow { ("flows.delete", it.flow_id.clone().unwrap_or_default()) } else { ("agents.delete", it.id.clone()) };
            fluxos::call(method, json!({"id": id}), |_| {});
        }
        for t in self.custom_tools.borrow().iter().filter(|t| ui::made_by_agent(t["created_by"].as_str().unwrap_or(""))) {
            fluxos::call("tools.delete", json!({"id": t["id"]}), |_| {});
        }
        // (the calls run in order on their threads: reading again a moment later sees them done)
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
            if let Some(l) = w.upgrade() {
                l.chosen.borrow_mut().clear();
                l.reload();
            }
        });
    }
}

/// The chips under a card's description.
fn card_chips(a: &Value, bare_flow: bool) -> Vec<String> {
    let c = &a["config"];
    match ui::kind_of(a) {
        "assistant" => {
            let n = c["tools"].as_array().map(|t| t.len()).unwrap_or(0);
            let model = if c["model_mode"].as_str().unwrap_or("auto") == "auto" { tr("LFM3 local").to_string() } else { ui::model_label(&c["model"]) };
            let mut v = vec![model];
            if n > 0 {
                v.push(if n == 1 { tr("1 tool").to_string() } else { trf("{n} tools", &[("n", n.to_string())]) });
            }
            v
        }
        "sequential" | "parallel" | "loop" => {
            let n = c["sub_agents"].as_array().map(|t| t.len()).unwrap_or(0);
            let mut v = vec![if n == 1 { tr("1 sub-agent").to_string() } else { trf("{n} sub-agents", &[("n", n.to_string())]) }];
            if ui::kind_of(a) == "loop" {
                v.push(trf("up to {n} times", &[("n", c["max_iterations"].as_u64().unwrap_or(5).to_string())]));
            }
            v
        }
        "remote" => vec![ui::split_url(c["card_url"].as_str().unwrap_or("")).0, "A2A".into()],
        // (a task list said "fluxo")
        "tasks" => {
            let n = c["tasks"].as_array().map(|t| t.len()).unwrap_or(0);
            vec![if n == 1 { tr("1 task").to_string() } else { trf("{n} tasks", &[("n", n.to_string())]) }]
        }
        _ => vec![if bare_flow { tr("flow without agent").to_string() } else { tr("flow").to_string() }],
    }
}

/// What a card says when the agent has no description: the start of its instructions.
fn default_description(a: &Value) -> String {
    let i = a["config"]["instruction"].as_str().unwrap_or("");
    let first = i.lines().next().unwrap_or("").trim();
    if first.is_empty() { tr("No description.").to_string() } else { first.to_string() }
}

/// "Exportar": the agent as a JSON file.
pub fn export_agent(w: &impl IsA<gtk::Widget>, agent: &Value) {
    let d = gtk::FileDialog::new();
    d.set_title(tr("Export agent"));
    d.set_initial_name(Some(&format!("{}.json", crate::central_ajustes::nome_valido(agent["name"].as_str().unwrap_or("agente")))));
    let text = serde_json::to_string_pretty(&json!({"agent": agent})).unwrap_or_default();
    d.save(w.as_ref().root().and_downcast_ref::<gtk::Window>(), gtk::gio::Cancellable::NONE, move |r| {
        if let Ok(f) = r {
            if let Some(p) = f.path() {
                if let Err(e) = std::fs::write(&p, &text) {
                    eprintln!("kalango-shell: exportar agente: {e}");
                }
            }
        }
    });
}

/// Whether a run is this item's: its flow's (a flow agent, or a flow no agent wraps), or the agent's own (a
/// conversation: kalango-fluxos' runs carry the agent's id) — chat-used agents said "nunca rodou" before.
fn run_belongs(r: &Value, it: &Item) -> bool {
    let flow = it.flow_id.as_deref().is_some_and(|f| r["flow_id"].as_str() == Some(f));
    flow || (!it.bare_flow && r["agent_id"].as_str() == Some(it.id.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_list_card_counts_its_tasks() {
        let a = json!({"kind": "tasks", "config": {"tasks": [{"agent": "x"}, {"agent": "y"}]}});
        assert_eq!(card_chips(&a, false), vec![trf("{n} tasks", &[("n", "2".to_string())])]);
    }

    #[test]
    fn a_conversation_run_is_the_agents_own() {
        let it = |id: &str, flow: Option<&str>, bare: bool| Item { id: id.into(), agent: json!({}), created_by: "person".into(), flow_id: flow.map(str::to_string), bare_flow: bare };
        let chat = json!({"agent_id": "a1", "flow_id": null});
        assert!(run_belongs(&chat, &it("a1", None, false)));
        assert!(!run_belongs(&chat, &it("a2", None, false)));
        let flow = json!({"agent_id": null, "flow_id": "f1"});
        assert!(run_belongs(&flow, &it("ag", Some("f1"), false)) && run_belongs(&flow, &it("f1", Some("f1"), true)));
    }

    #[test]
    fn counts_tools_by_origin() {
        let names = ["arquivos.listar", "arquivos.mover", "correio.enviar", "clima.previsao"];
        let c = tool_counts(&names, &["correio".into()], &["clima.previsao".into()]);
        assert_eq!(c, (2, 1, 1));
        assert_eq!(tools_summary((0, 0, 0)), tr("none"));
    }

    #[test]
    fn facts_of_each_kind() {
        let a = json!({"kind": "assistant", "config": {"instruction": "x", "tools": [{"name": "arquivos.listar", "permission": "free"}, {"name": "arquivos.apagar"}], "reply_with_voice": true}});
        let f = facts(&a, &|s| s.to_string(), &[], &[]);
        assert_eq!(f.len(), 6);
        assert_eq!(f[4].1, trf("asks before {n} tools", &[("n", "1".into())]));
        assert_eq!(f[5].1, tr("text and voice"));
        let t = json!({"kind": "loop", "config": {"sub_agents": ["a", "b"], "max_iterations": 3}});
        let f = facts(&t, &|s| s.to_uppercase(), &[], &[]);
        assert_eq!(f[1].1, "A → B");
        assert_eq!(f[2].1, trf("up to {n} times", &[("n", "3".into())]));
        assert_eq!(facts(&json!({"kind": "flow", "config": {"flow_id": "f1"}}), &|_| "Fotos".into(), &[], &[])[0].1, "Fotos");
    }

    #[test]
    fn chips_of_cards() {
        assert_eq!(card_chips(&json!({"kind": "remote", "config": {"card_url": "https://sala.local/agent.json"}}), false), vec!["sala.local".to_string(), "A2A".into()]);
        assert_eq!(card_chips(&json!({"kind": "sequential", "config": {"sub_agents": ["a", "b", "c"]}}), false)[0], trf("{n} sub-agents", &[("n", "3".into())]));
        assert_eq!(default_description(&json!({"config": {"instruction": "Organize.\nMais."}})), "Organize.");
    }
}
