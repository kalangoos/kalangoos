//! Agents center › Rotinas › a flow (design "Fluxo · construtor"): the flow of kalango-fluxos on a canvas. The top
//! bar names it (active or paused, its trigger, the last run), switches between Editar and Execuções, tests it with
//! data and runs it now (`flows.run`). Editar holds the palette (triggers, agents, logic, multimodal, actions: a
//! click adds after the chosen block, a drag drops where it lands), the canvas (`fluxo_canvas`) and the inspector
//! (`fluxo_inspetor`). Every change is checked by the service (`flows.validate`, the problems shown on their blocks)
//! and saved a moment later (`flows.save`); triggers go by `triggers.save`. Execuções and the live run are
//! `fluxo_execucoes`.
//!
//! The service keeps a block's kind, data and place but not the name the person gave it: the names are kept here,
//! per flow, in the shell's data folder.

use crate::conversa::ui_icon;
use crate::fluxo_canvas::{Canvas, CanvasData, CanvasEvent};
use crate::fluxo_execucoes::{Live, RunsView};
use crate::fluxo_modelo::{self as m, EdgeDoc, FlowDoc, NodeDoc, Rect, Step, TriggerDoc};
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

/// "hoje 09:12", "ontem 18:40", "seg 08:15", "03/11 10:00".
pub fn when_text(t: i64) -> String {
    let (Ok(d), Ok(now)) = (glib::DateTime::from_unix_local(t), glib::DateTime::now_local()) else { return String::new() };
    let hm = d.format("%H:%M").map(|s| s.to_string()).unwrap_or_default();
    let day = |x: &glib::DateTime| glib::DateTime::from_local(x.year(), x.month(), x.day_of_month(), 0, 0, 0.0).map(|y| y.to_unix()).unwrap_or(0);
    let days = (day(&now) - day(&d)) / 86_400;
    let when = match days {
        0 => tr("today").to_string(),
        1 => tr("yesterday").to_string(),
        -1 => tr("tomorrow").to_string(),
        2..=6 => d.format("%a").map(|s| s.trim_end_matches('.').to_lowercase()).unwrap_or_default(),
        _ => d.format("%d/%m").map(|s| s.to_string()).unwrap_or_default(),
    };
    format!("{when} {hm}")
}

/// A run's state as the screens name it, with its css class.
pub fn run_status_text(status: &str) -> (String, &'static str) {
    match status {
        "done" => (tr("completed").into(), "ok"),
        "failed" => (tr("failed").into(), "falhou"),
        "stopped" => (tr("stopped").into(), "neutro"),
        "denied" => (tr("refused by you").into(), "falhou"),
        "waiting_approval" | "waiting_input" => (tr("waits for you").into(), "aguarda"),
        "running" | "pending" => (tr("running").into(), "rodando"),
        s => (s.to_string(), "neutro"),
    }
}

// ── the names the person gave the blocks (kept by the shell) ─────────────────────────────────────────────────────

/// How kalango-fluxos begins the refusal of a run's input (`runtime::INVALID_INPUT`); the rest is its pt-BR sentence.
const INVALID_INPUT: &str = "entrada inválida: ";

fn names_path() -> std::path::PathBuf {
    glib::user_data_dir().join("kalango/fluxos-nomes.json")
}

fn load_names(flow: &str) -> HashMap<String, String> {
    std::fs::read_to_string(names_path())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v[flow].as_object().map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()))
        .unwrap_or_default()
}

fn save_names(flow: &str, doc: &FlowDoc) {
    let path = names_path();
    let mut all: Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(json!({}));
    if !all.is_object() {
        all = json!({});
    }
    let names: serde_json::Map<String, Value> = doc.nodes.iter().filter(|n| !n.title.is_empty()).map(|n| (n.id.clone(), json!(n.title))).collect();
    all[flow] = Value::Object(names);
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&path, serde_json::to_string_pretty(&all).unwrap_or_default());
}

/// A fresh id for a trigger made on this screen.
fn new_trigger_id() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("g{:x}", t & 0xffff_ffff_ffff)
}

pub struct FlowPage {
    pub root: gtk::Box,
    pub flow_id: RefCell<String>,
    pub name: RefCell<String>,
    pub enabled: Cell<bool>,
    pub doc: RefCell<FlowDoc>,
    pub triggers: RefCell<Vec<TriggerDoc>>,
    /// (id, name)
    pub agents: RefCell<Vec<(String, String)>>,
    pub flows: RefCell<Vec<(String, String)>>,
    pub tools: RefCell<Vec<String>>,
    /// the recent runs (newest first) with their steps
    pub recent: RefCell<Vec<(Value, Vec<Step>)>>,
    /// every run of the flow (`runs.list`), newest first
    pub runs: RefCell<Vec<Value>>,
    pub selected: RefCell<Option<String>>,
    pub insp_tab: RefCell<String>,
    /// problems per block id ("" for the flow; "trigger:<id>" for a trigger)
    pub issues: RefCell<HashMap<String, String>>,
    pub narrow: Cell<bool>,
    pub insp_content: gtk::Box,
    pub insp_footer: gtk::Box,
    /// the chosen block's problem line in the inspector
    pub issue_label: RefCell<Option<(String, gtk::Label)>>,
    /// the inspector's mapping rows: they show their problems again each time the flow is checked or saved
    pub on_issues: RefCell<Option<Rc<dyn Fn()>>>,
    pub inspector: gtk::Box,
    pub canvas: Rc<Canvas>,
    pub live: RefCell<Option<Live>>,
    pub runs_view: RefCell<Option<Rc<RunsView>>>,
    pub body: gtk::Stack,
    palette: gtk::Box,
    palette_list: gtk::Box,
    search: gtk::Entry,
    title: gtk::Label,
    state_chip: gtk::Label,
    sub_trigger: gtk::Label,
    sub_last: gtk::Label,
    sub_save: gtk::Label,
    seg_edit: gtk::Button,
    seg_runs: gtk::Button,
    test_btn: gtk::MenuButton,
    run_btn: gtk::Button,
    header_actions: gtk::Box,
    top: gtk::Box,
    save_timer: RefCell<Option<glib::SourceId>>,
    trigger_timers: RefCell<HashMap<String, glib::SourceId>>,
    /// a palette item chosen to go into the connection picked on the canvas ("/")
    insert_into: RefCell<Option<EdgeDoc>>,
    /// loads still answering for an older flow are dropped
    generation: Cell<u64>,
    /// the flow and its triggers still answering (the canvas fits the whole once both are in)
    pending: Cell<u8>,
    /// the breadcrumb's "Rotinas" and the timeline (set by the center)
    pub go: RefCell<Option<Rc<dyn Fn(&str)>>>,
    /// what was typed in "Testar com dados", by field (kept while the flow is open)
    test_values: RefCell<HashMap<String, String>>,
    /// the service's last refusal of a run's input ("entrada inválida: …"), shown as it came
    pub run_refusal: RefCell<Option<String>>,
}

impl FlowPage {
    pub fn new() -> Rc<FlowPage> {
        let root = caixa(Orientation::Vertical, 0, "fluxo-pagina");
        root.set_vexpand(true);
        root.set_hexpand(true);
        // the top bar
        let top = caixa(Orientation::Horizontal, 12, "fluxo-topo");
        let names = caixa(Orientation::Vertical, 3, "");
        names.set_hexpand(true);
        names.set_valign(Align::Center);
        let l1 = caixa(Orientation::Horizontal, 8, "");
        let title = rotulo("", "fluxo-nome");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_xalign(0.0);
        l1.append(&title);
        let state_chip = rotulo("", "fluxo-estado");
        state_chip.set_valign(Align::Center);
        l1.append(&state_chip);
        names.append(&l1);
        let l2 = caixa(Orientation::Horizontal, 6, "");
        let sub_trigger = rotulo("", "fluxo-sub");
        sub_trigger.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l2.append(&sub_trigger);
        let sub_dot = rotulo("·", "fluxo-sub");
        l2.append(&sub_dot);
        let sub_last = rotulo("", "fluxo-sub");
        sub_last.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l2.append(&sub_last);
        let sub_save = rotulo("", "fluxo-sub");
        sub_save.add_css_class("atencao");
        sub_save.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l2.append(&sub_save);
        names.append(&l2);
        top.append(&names);
        let header_actions = caixa(Orientation::Horizontal, 10, "");
        header_actions.set_valign(Align::Center);
        let seg = caixa(Orientation::Horizontal, 0, "fluxo-seg");
        seg.add_css_class("topo");
        let seg_edit = gtk::Button::with_label(tr("Edit"));
        seg_edit.add_css_class("fluxo-seg-item");
        seg_edit.add_css_class("ativo");
        let seg_runs = gtk::Button::with_label(tr("Runs"));
        seg_runs.add_css_class("fluxo-seg-item");
        seg.append(&seg_edit);
        seg.append(&seg_runs);
        header_actions.append(&seg);
        let test_btn = gtk::MenuButton::new();
        test_btn.add_css_class("fluxo-botao");
        let tc = caixa(Orientation::Horizontal, 8, "");
        tc.append(&ui_icon("olho", "#E2E2E6", 16));
        tc.append(&rotulo(tr("Test with data"), ""));
        test_btn.set_child(Some(&tc));
        header_actions.append(&test_btn);
        let run_btn = gtk::Button::new();
        run_btn.add_css_class("fluxo-botao");
        run_btn.add_css_class("primario");
        let rc = caixa(Orientation::Horizontal, 8, "");
        rc.append(&ui_icon("tocar", "#1C1D24", 14));
        rc.append(&rotulo(tr("Run it now"), ""));
        run_btn.set_child(Some(&rc));
        header_actions.append(&run_btn);
        top.append(&header_actions);
        root.append(&top);
        let top = top.clone();

        // the body: Editar (palette, canvas, inspector) and Execuções
        let body = gtk::Stack::new();
        body.set_vexpand(true);
        body.set_hhomogeneous(false);
        body.set_transition_type(gtk::StackTransitionType::Crossfade);
        body.set_transition_duration(120);
        let edit = caixa(Orientation::Horizontal, 0, "");
        // palette
        let palette = caixa(Orientation::Vertical, 0, "fluxo-paleta");
        palette.set_size_request(196, -1);
        palette.set_hexpand(false);
        let busca = caixa(Orientation::Horizontal, 7, "fluxo-busca");
        busca.append(&ui_icon("busca", "#8E8F99", 14));
        let search = gtk::Entry::new();
        search.add_css_class("fluxo-busca-campo");
        search.set_placeholder_text(Some(tr("Search blocks…")));
        search.set_hexpand(true);
        busca.append(&search);
        palette.append(&busca);
        let palette_list = caixa(Orientation::Vertical, 0, "");
        let ps = gtk::ScrolledWindow::new();
        ps.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        ps.set_vexpand(true);
        ps.set_child(Some(&palette_list));
        palette.append(&ps);
        edit.append(&palette);
        let canvas = Canvas::new();
        edit.append(&canvas.root);
        // inspector
        let inspector = caixa(Orientation::Vertical, 0, "fluxo-insp");
        inspector.set_size_request(338, -1);
        inspector.set_hexpand(false);
        let insp_content = caixa(Orientation::Vertical, 0, "fluxo-insp-conteudo");
        let is = gtk::ScrolledWindow::new();
        is.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        is.set_vexpand(true);
        is.set_child(Some(&insp_content));
        inspector.append(&is);
        let insp_footer = caixa(Orientation::Horizontal, 8, "fluxo-insp-rodape");
        inspector.append(&insp_footer);
        edit.append(&inspector);
        body.add_named(&edit, Some("editar"));
        root.append(&body);

        let p = Rc::new(FlowPage {
            root, flow_id: RefCell::default(), name: RefCell::default(), enabled: Cell::new(true), doc: RefCell::default(), triggers: RefCell::default(),
            agents: RefCell::default(), flows: RefCell::default(), tools: RefCell::default(), recent: RefCell::default(), runs: RefCell::default(),
            selected: RefCell::default(), insp_tab: RefCell::new("ajustes".into()), issues: RefCell::default(), narrow: Cell::new(false), insp_content, insp_footer,
            issue_label: RefCell::default(), on_issues: RefCell::default(), inspector, canvas, live: RefCell::default(), runs_view: RefCell::default(), body, palette, palette_list, search, title, state_chip, sub_trigger, sub_last,
            sub_save, seg_edit, seg_runs, test_btn, run_btn, header_actions, top, save_timer: RefCell::default(), trigger_timers: RefCell::default(), insert_into: RefCell::default(),
            generation: Cell::new(0), pending: Cell::new(0), go: RefCell::default(), test_values: RefCell::default(), run_refusal: RefCell::default(),
        });
        let rv = RunsView::new(&p);
        p.body.add_named(&rv.root, Some("execucoes"));
        *p.runs_view.borrow_mut() = Some(rv);
        p.build_palette();
        let w = Rc::downgrade(&p);
        p.search.connect_changed(move |_| {
            if let Some(p) = w.upgrade() {
                p.build_palette();
            }
        });
        let w = Rc::downgrade(&p);
        *p.canvas.on_event.borrow_mut() = Some(Rc::new(move |e| {
            if let Some(p) = w.upgrade() {
                p.on_canvas(e);
            }
        }));
        // keys on the canvas: Delete removes the chosen block or connection; "/" searches the palette
        let keys = gtk::EventControllerKey::new();
        let w = Rc::downgrade(&p);
        keys.connect_key_pressed(move |_, k, _, _| {
            let Some(p) = w.upgrade() else { return glib::Propagation::Proceed };
            match k {
                gdk::Key::Delete | gdk::Key::BackSpace => {
                    let edge = p.canvas.edge.borrow_mut().take();
                    if let Some(e) = edge {
                        p.edit_flow(|f| f.edges.retain(|x| *x != e));
                    } else if p.selected.borrow().is_some() {
                        p.remove_selected();
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::slash => {
                    *p.insert_into.borrow_mut() = p.canvas.edge.borrow().clone();
                    p.search.grab_focus();
                    glib::Propagation::Stop
                }
                gdk::Key::Escape => {
                    p.select(None);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        p.canvas.root.add_controller(keys);
        for (b, page) in [(&p.seg_edit, "editar"), (&p.seg_runs, "execucoes")] {
            let w = Rc::downgrade(&p);
            b.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    p.show_mode(page);
                }
            });
        }
        let w = Rc::downgrade(&p);
        p.run_btn.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                // (a flow with input fields asks for them first)
                if p.doc.borrow().inputs().is_empty() {
                    p.run("");
                } else {
                    p.test_btn.popup();
                }
            }
        });
        p.build_test_popover();
        crate::fluxo_execucoes::listen(&p);
        p
    }

    // ── loading ─────────────────────────────────────────────────────────────────────────────────────────────────

    /// Opens a flow of the service (the canvas shows it once its parts answer).
    pub fn open(self: &Rc<Self>, id: &str) {
        let gen = self.generation.get() + 1;
        self.generation.set(gen);
        *self.flow_id.borrow_mut() = id.to_string();
        *self.selected.borrow_mut() = None;
        *self.insp_tab.borrow_mut() = "ajustes".into();
        *self.live.borrow_mut() = None;
        self.issues.borrow_mut().clear();
        self.test_values.borrow_mut().clear();
        *self.run_refusal.borrow_mut() = None;
        self.recent.borrow_mut().clear();
        self.runs.borrow_mut().clear();
        self.triggers.borrow_mut().clear();
        *self.doc.borrow_mut() = FlowDoc::default();
        self.canvas.reset_fit();
        self.pending.set(2);
        self.show_mode("editar");
        let w = Rc::downgrade(self);
        crate::fluxos::call("flows.get", json!({"id": id}), move |r| {
            let Some(p) = w.upgrade().filter(|p| p.generation.get() == gen) else { return };
            match r {
                Ok(v) => {
                    *p.name.borrow_mut() = v["name"].as_str().unwrap_or("").to_string();
                    let mut doc = FlowDoc::from_json(&v["flow"]);
                    let names = load_names(&p.flow_id.borrow());
                    for n in doc.nodes.iter_mut() {
                        if let Some(t) = names.get(&n.id) {
                            n.title = t.clone();
                        }
                    }
                    *p.doc.borrow_mut() = doc;
                }
                Err(e) => {
                    p.issues.borrow_mut().insert(String::new(), e);
                }
            }
            p.loaded();
        });
        self.reload_lists();
        let w = Rc::downgrade(self);
        crate::fluxos::call("triggers.list", json!({"flow_id": id}), move |r| {
            let Some(p) = w.upgrade().filter(|p| p.generation.get() == gen) else { return };
            *p.triggers.borrow_mut() = r
                .unwrap_or_default()
                .as_array()
                .into_iter()
                .flatten()
                .map(|t| TriggerDoc { id: t["id"].as_str().unwrap_or("").into(), config: t["config"].clone(), enabled: t["enabled"].as_bool().unwrap_or(true), saved: true })
                .collect();
            p.loaded();
        });
        self.reload_runs();
    }

    /// One of the flow's parts answered: once all are in, the canvas shows the whole flow.
    fn loaded(self: &Rc<Self>) {
        self.pending.set(self.pending.get().saturating_sub(1));
        if self.pending.get() == 0 {
            self.canvas.reset_fit();
        }
        self.refresh();
    }

    /// The agents, the other flows, the system actions (for the inspector's choices) and this flow's state.
    fn reload_lists(self: &Rc<Self>) {
        let gen = self.generation.get();
        let w = Rc::downgrade(self);
        crate::fluxos::call("agents.list", json!({}), move |r| {
            let Some(p) = w.upgrade().filter(|p| p.generation.get() == gen) else { return };
            *p.agents.borrow_mut() = r
                .unwrap_or_default()
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| Some((a["agent"]["id"].as_str()?.to_string(), a["agent"]["name"].as_str().unwrap_or("").to_string())))
                .collect();
            p.refresh();
        });
        let w = Rc::downgrade(self);
        crate::fluxos::call("flows.list", json!({}), move |r| {
            let Some(p) = w.upgrade().filter(|p| p.generation.get() == gen) else { return };
            let list = r.unwrap_or_default();
            let me = p.flow_id.borrow().clone();
            if let Some(f) = list.as_array().into_iter().flatten().find(|f| f["id"] == json!(me)) {
                p.enabled.set(f["enabled"].as_bool().unwrap_or(true));
            }
            *p.flows.borrow_mut() = list.as_array().into_iter().flatten().filter_map(|f| Some((f["id"].as_str()?.to_string(), f["name"].as_str().unwrap_or("").to_string()))).collect();
            p.refresh_header();
        });
        let w = Rc::downgrade(self);
        crate::fluxos::call("tools.list", json!({}), move |r| {
            let Some(p) = w.upgrade().filter(|p| p.generation.get() == gen) else { return };
            let v = r.unwrap_or_default();
            *p.tools.borrow_mut() = v["system"].as_array().into_iter().flatten().chain(v["custom"].as_array().into_iter().flatten()).filter_map(|t| t["name"].as_str().or(t["id"].as_str()).map(str::to_string)).collect();
        });
    }

    /// The flow's runs and the steps of the recent ones.
    pub fn reload_runs(self: &Rc<Self>) {
        let gen = self.generation.get();
        let id = self.flow_id.borrow().clone();
        let w = Rc::downgrade(self);
        crate::fluxos::call("runs.list", json!({"flow_id": id, "limit": 200}), move |r| {
            let Some(p) = w.upgrade().filter(|p| p.generation.get() == gen) else { return };
            let runs: Vec<Value> = r.unwrap_or_default().as_array().cloned().unwrap_or_default();
            *p.runs.borrow_mut() = runs.clone();
            p.refresh_header();
            if let Some(rv) = p.runs_view.borrow().clone() {
                rv.render();
            }
            // the steps of the last 8 runs (the canvas marks the last one, the inspector reads them all)
            let ids: Vec<String> = runs.iter().take(8).filter_map(|r| r["id"].as_str().map(str::to_string)).collect();
            if ids.is_empty() {
                p.recent.borrow_mut().clear();
                p.refresh();
                return;
            }
            let w = Rc::downgrade(&p);
            let n = ids.len();
            let got: Rc<RefCell<Vec<Option<(Value, Vec<Step>)>>>> = Rc::new(RefCell::new(vec![None; n]));
            let left = Rc::new(Cell::new(n));
            for (i, id) in ids.into_iter().enumerate() {
                let (w, got, left) = (w.clone(), got.clone(), left.clone());
                crate::fluxos::call("runs.get", json!({"id": id}), move |r| {
                    if let Ok(v) = r {
                        let steps = v["steps"].as_array().into_iter().flatten().map(Step::from_json).collect();
                        got.borrow_mut()[i] = Some((v["run"].clone(), steps));
                    }
                    left.set(left.get() - 1);
                    if left.get() == 0 {
                        if let Some(p) = w.upgrade().filter(|p| p.generation.get() == gen) {
                            *p.recent.borrow_mut() = got.borrow_mut().drain(..).flatten().collect();
                            p.refresh();
                        }
                    }
                });
            }
        });
    }

    // ── showing ─────────────────────────────────────────────────────────────────────────────────────────────────

    pub fn show_mode(self: &Rc<Self>, mode: &str) {
        self.body.set_visible_child_name(mode);
        for (b, m) in [(&self.seg_edit, "editar"), (&self.seg_runs, "execucoes")] {
            if m == mode { b.add_css_class("ativo") } else { b.remove_css_class("ativo") }
        }
        // (Execuções has its own header: the top bar belongs to the builder)
        let runs = mode == "execucoes";
        self.update_top();
        if runs {
            if let Some(rv) = self.runs_view.borrow().clone() {
                // (coming in: the newest run first)
                rv.reset_choice();
                rv.render();
            }
        }
    }

    pub fn is_runs_mode(&self) -> bool {
        self.body.visible_child_name().as_deref() == Some("execucoes")
    }

    /// The top bar, the canvas and the inspector, from the flow as it is now.
    pub fn refresh(self: &Rc<Self>) {
        self.refresh_header();
        self.canvas.set_data(self.canvas_data());
        self.refresh_inspector();
        // the canvas' block names (and the inspector's) follow the agents' names
    }

    pub fn refresh_inspector(self: &Rc<Self>) {
        if self.live.borrow().as_ref().is_some_and(|l| l.active()) {
            crate::fluxo_execucoes::live_panel(self);
        } else {
            crate::fluxo_inspetor::build(self);
        }
    }

    pub fn refresh_header(self: &Rc<Self>) {
        self.title.set_text(&self.name.borrow());
        let live = self.live.borrow().clone();
        self.state_chip.remove_css_class("pausado");
        self.state_chip.remove_css_class("rodando");
        self.sub_last.remove_css_class("ok");
        self.sub_last.remove_css_class("falhou");
        self.sub_last.remove_css_class("aguarda");
        if let Some(l) = live.as_ref().filter(|l| l.active()) {
            let secs = (glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0) - l.started).max(0);
            self.state_chip.set_text(&format!("{} · {:02}:{:02}", tr("running"), secs / 60, secs % 60));
            self.state_chip.add_css_class("rodando");
            let total = self.doc.borrow().nodes.len();
            let step = l.steps.len().max(1);
            let mut t = trf("step {n} of {total}", &[("n", step.to_string()), ("total", total.max(step).to_string())]);
            if l.waiting.is_some() {
                t.push_str(&format!(" · {}", tr("waits for you")));
                self.sub_last.add_css_class("aguarda");
            } else {
                self.sub_last.add_css_class("ok");
            }
            self.sub_last.set_text(&t);
        } else {
            self.state_chip.set_text(if self.enabled.get() { tr("active") } else { tr("paused") });
            if !self.enabled.get() {
                self.state_chip.add_css_class("pausado");
            }
            match self.runs.borrow().first() {
                Some(r) => {
                    let (st, class) = run_status_text(r["status"].as_str().unwrap_or(""));
                    let st = if class == "ok" { "ok".to_string() } else { st };
                    self.sub_last.set_text(&format!("{}: {} · {st}", tr("last"), when_text(r["created_at"].as_i64().unwrap_or(0))));
                    self.sub_last.add_css_class(if class == "ok" { "ok" } else if class == "aguarda" { "aguarda" } else { "falhou" });
                }
                None => self.sub_last.set_text(tr("never ran")),
            }
        }
        let trig = self.triggers.borrow().first().map(|t| m::trigger_summary(&t.config)).unwrap_or_else(|| tr("by hand").to_string());
        self.sub_trigger.set_text(&format!("{}: {trig}", tr("trigger")));
        let n = self.runs.borrow().len();
        self.seg_runs.set_label(&if n > 0 { format!("{} {n}", tr("Runs")) } else { tr("Runs").to_string() });
        let issues = self.issues.borrow().len();
        self.sub_save.set_visible(issues > 0);
        if issues > 0 {
            self.sub_save.set_text(&format!("· {}", if issues == 1 { tr("1 problem: not saved").to_string() } else { trf("{n} problems: not saved", &[("n", issues.to_string())]) }));
        }
    }

    /// What the canvas draws: the flow, its triggers, the run shown (the live one, else the last) and the problems.
    pub fn canvas_data(&self) -> CanvasData {
        let flow = self.doc.borrow().clone();
        let live = self.live.borrow().clone();
        let (steps, waiting): (Vec<Step>, Option<String>) = match live.as_ref().filter(|l| l.active() || l.finished_shown) {
            Some(l) => (l.steps.clone(), l.waiting.as_ref().map(|w| w.node.clone())),
            None => (self.recent.borrow().first().map(|(_, s)| s.clone()).unwrap_or_default(), None),
        };
        let (states, travelled) = m::run_marks(&flow, &steps, waiting.as_deref());
        let mut items = HashMap::new();
        let mut labels = HashMap::new();
        for s in &steps {
            if s.kind == "for_each" {
                if let Some(k) = s.detail.split_whitespace().next().and_then(|x| x.parse::<usize>().ok()) {
                    items.insert(s.node.clone(), k);
                }
            }
            if s.kind == "decide" {
                let route = flow.node(&s.node).and_then(|n| n.data["route"].as_str().map(str::to_string)).unwrap_or_default();
                if let Some(d) = m::parse_decision(s, &route) {
                    if let Some(p) = d.options.iter().find(|o| o.0 == d.chosen).map(|o| o.1) {
                        let kind = "decide";
                        let name = m::way_label(kind, Some(&d.chosen));
                        labels.insert((s.node.clone(), d.chosen.clone()), format!("{name} {}%", (p * 100.0).round()));
                    }
                }
            }
        }
        CanvasData {
            flow,
            triggers: self.triggers.borrow().clone(),
            agents: self.agent_names(),
            states,
            travelled,
            items,
            labels,
            issues: self.issues.borrow().clone(),
            selected: self.selected.borrow().clone(),
        }
    }

    pub fn agent_names(&self) -> HashMap<String, String> {
        self.agents.borrow().iter().cloned().collect()
    }

    pub fn node_title(&self, id: &str) -> String {
        if let Some(t) = id.strip_prefix(m::TRIGGER_PREFIX) {
            return self.triggers.borrow().iter().find(|x| x.id == t).map(|x| m::trigger_text(&x.config).0).unwrap_or_default();
        }
        self.doc.borrow().node(id).map(|n| m::node_text(n, &self.agent_names(), None).0).unwrap_or_else(|| id.to_string())
    }

    pub fn last_step(&self, node: &str) -> Option<Step> {
        if let Some(l) = self.live.borrow().as_ref() {
            if let Some(s) = crate::fluxo_inspetor::last_of(&l.steps, node) {
                return Some(s);
            }
        }
        self.recent.borrow().first().and_then(|(_, s)| crate::fluxo_inspetor::last_of(s, node))
    }

    pub fn select(self: &Rc<Self>, id: Option<String>) {
        if *self.selected.borrow() != id {
            *self.insp_tab.borrow_mut() = "ajustes".into();
        }
        *self.selected.borrow_mut() = id;
        self.canvas.set_data(self.canvas_data());
        self.refresh_inspector();
    }

    // ── the palette ─────────────────────────────────────────────────────────────────────────────────────────────

    fn build_palette(self: &Rc<Self>) {
        while let Some(c) = self.palette_list.first_child() {
            self.palette_list.remove(&c);
        }
        let q = self.search.text().to_string();
        let mut any = false;
        for (group, items) in m::palette() {
            let items: Vec<_> = items.into_iter().filter(|(_, name, ..)| m::palette_matches(name, tr(name), &q)).collect();
            if items.is_empty() {
                continue;
            }
            any = true;
            let g = rotulo(tr(group), "fluxo-paleta-grupo");
            g.set_xalign(0.0);
            self.palette_list.append(&g);
            for (id, name, glyph, tone, badge) in items {
                let b = gtk::Button::new();
                b.add_css_class("fluxo-paleta-item");
                let row = caixa(Orientation::Horizontal, 10, "");
                let tile = caixa(Orientation::Horizontal, 0, "fluxo-paleta-tile");
                tile.add_css_class(tone.class());
                tile.set_size_request(22, 22);
                tile.set_valign(Align::Center);
                let gl = if glyph == "spark" { crate::superficies::estrela(11, tone.color()).upcast::<gtk::Widget>() } else { ui_icon(glyph, tone.color(), 14).upcast() };
                gl.set_hexpand(true);
                tile.append(&gl);
                tile.set_hexpand(false);
                row.append(&tile);
                let l = rotulo(tr(name), "fluxo-paleta-nome");
                l.set_xalign(0.0);
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                l.set_hexpand(true);
                row.append(&l);
                if !badge.is_empty() {
                    let bd = rotulo(badge, "fluxo-paleta-selo");
                    bd.set_valign(Align::Center);
                    row.append(&bd);
                }
                b.set_child(Some(&row));
                b.set_tooltip_text(Some(tr("Click to add after the chosen block, or drag onto the canvas")));
                let w = Rc::downgrade(self);
                b.connect_clicked(move |_| {
                    if let Some(p) = w.upgrade() {
                        p.add_item(id, None);
                    }
                });
                let src = gtk::DragSource::new();
                src.set_actions(gdk::DragAction::COPY);
                src.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&id.to_value())));
                b.add_controller(src);
                self.palette_list.append(&b);
            }
        }
        if !any {
            let l = rotulo(tr("No block with this name."), "fluxo-chave-sub");
            l.set_xalign(0.0);
            l.set_margin_top(12);
            self.palette_list.append(&l);
        }
    }

    // ── editing ─────────────────────────────────────────────────────────────────────────────────────────────────

    fn on_canvas(self: &Rc<Self>, e: CanvasEvent) {
        match e {
            CanvasEvent::Select(id) => self.select(id),
            CanvasEvent::Moved(id, x, y) => {
                if let Some(n) = self.doc.borrow_mut().node_mut(&id) {
                    n.x = x;
                    n.y = y;
                }
                self.schedule_save();
            }
            CanvasEvent::Connect(a, b) => {
                let ok = self.doc.borrow_mut().connect(&a, &b);
                if ok {
                    self.changed();
                }
            }
            CanvasEvent::Drop(item, at) => self.add_item(&item, Some(at)),
            // (a connection chosen: the block is no longer)
            CanvasEvent::Edge(Some(_)) if self.selected.borrow().is_some() => self.select(None),
            CanvasEvent::Edge(_) => {}
        }
    }

    /// A palette item: a trigger, or a block (after the chosen one, at `at`, or into the connection picked).
    pub fn add_item(self: &Rc<Self>, item: &str, at: Option<(f64, f64)>) {
        if let Some(kind) = item.strip_prefix("trigger:") {
            let id = new_trigger_id();
            let cfg = m::default_trigger(kind, &self.flow_id.borrow());
            self.triggers.borrow_mut().push(TriggerDoc { id: id.clone(), config: cfg, enabled: true, saved: false });
            self.save_trigger(&id);
            self.select(Some(format!("{}{id}", m::TRIGGER_PREFIX)));
            return;
        }
        let Some(rest) = item.strip_prefix("block:") else { return };
        let (kind, preset) = rest.split_once(':').unwrap_or((rest, ""));
        let into = self.insert_into.borrow_mut().take().or_else(|| self.canvas.edge.borrow().clone());
        let rects = self.canvas.rects();
        let w = m::node_width(kind);
        let (x, y, after) = match (at, &into) {
            (Some(p), _) => (p.0 - w / 2.0, p.1 - 30.0, None),
            (None, Some(e)) => {
                let (a, b) = (rects.iter().find(|r| r.0 == e.source).map(|r| r.1), rects.iter().find(|r| r.0 == e.target).map(|r| r.1));
                match (a, b) {
                    (Some(a), Some(b)) => {
                        let (sx, sy) = m::free_spot(&rects, Some(Rect { x: (a.x + b.x) / 2.0 - w / 2.0, y: (a.y + b.y) / 2.0, w: 0.0, h: 0.0 }), w, 90.0);
                        (sx, sy, None)
                    }
                    _ => (0.0, 0.0, None),
                }
            }
            (None, None) => {
                let sel = self.selected.borrow().clone().filter(|s| self.doc.borrow().node(s).is_some());
                let after = sel.as_ref().and_then(|s| rects.iter().find(|r| r.0 == *s).map(|r| r.1));
                // (with nothing chosen: where the view is, on a free spot)
                let (sx, sy) = match after {
                    Some(_) => m::free_spot(&rects, after, w, 90.0),
                    None if rects.is_empty() => (40.0, 60.0),
                    None => {
                        let c = self.canvas.center_world();
                        m::free_spot(&rects, Some(Rect { x: c.0 - w, y: c.1 - 45.0, w: 0.0, h: 0.0 }), w, 90.0)
                    }
                };
                (sx, sy, sel)
            }
        };
        let id = self.doc.borrow().new_id(kind);
        let mut data = m::default_data(kind, preset);
        // a new decision asks a question of its own (the inspector's ready uses fill it)
        if kind == "agent" && preset == "multimodal" {
            data["message"] = json!("");
        }
        let title = match preset {
            "team" => tr("Team").to_string(),
            "multimodal" => tr("Multimodal agent").to_string(),
            "repeat" => tr("Repeat until").to_string(),
            "mcp" => tr("MCP connector").to_string(),
            _ => String::new(),
        };
        let node = NodeDoc { id: id.clone(), x, y, kind: kind.to_string(), data, title };
        {
            let mut d = self.doc.borrow_mut();
            d.nodes.push(node);
            if let Some(e) = &into {
                d.edges.retain(|x| x != e);
                d.edges.push(EdgeDoc { source: e.source.clone(), target: id.clone(), handle: e.handle.clone() });
                d.edges.push(EdgeDoc { source: id.clone(), target: e.target.clone(), handle: None });
            } else if let Some(a) = after {
                d.connect(&a, &id);
            }
        }
        *self.canvas.edge.borrow_mut() = None;
        *self.selected.borrow_mut() = Some(id.clone());
        *self.insp_tab.borrow_mut() = "ajustes".into();
        self.changed();
        self.refresh_inspector();
        self.canvas.reveal(&id);
    }

    /// Changes a block (the inspector's fields): the canvas redraws, the flow is checked and saved.
    pub fn edit_node(self: &Rc<Self>, id: &str, f: impl FnOnce(&mut NodeDoc)) {
        if let Some(n) = self.doc.borrow_mut().node_mut(id) {
            f(n);
        }
        self.changed();
    }

    /// Changes the flow (connections): the canvas redraws, the flow is checked and saved.
    pub fn edit_flow(self: &Rc<Self>, f: impl FnOnce(&mut FlowDoc)) {
        f(&mut self.doc.borrow_mut());
        self.changed();
    }

    fn changed(self: &Rc<Self>) {
        self.canvas.set_data(self.canvas_data());
        self.schedule_save();
    }

    pub fn edit_trigger(self: &Rc<Self>, id: &str, f: impl FnOnce(&mut Value)) {
        if let Some(t) = self.triggers.borrow_mut().iter_mut().find(|t| t.id == id) {
            f(&mut t.config);
        }
        self.canvas.set_data(self.canvas_data());
        self.refresh_header();
        let w = Rc::downgrade(self);
        let id2 = id.to_string();
        if let Some(s) = self.trigger_timers.borrow_mut().remove(id) {
            s.remove();
        }
        let s = glib::timeout_add_local_once(Duration::from_millis(600), move || {
            if let Some(p) = w.upgrade() {
                p.trigger_timers.borrow_mut().remove(&id2);
                p.save_trigger(&id2);
            }
        });
        self.trigger_timers.borrow_mut().insert(id.to_string(), s);
    }

    fn save_trigger(self: &Rc<Self>, id: &str) {
        let Some(t) = self.triggers.borrow().iter().find(|t| t.id == id).cloned() else { return };
        let key = format!("{}{id}", m::TRIGGER_PREFIX);
        let w = Rc::downgrade(self);
        let fid = self.flow_id.borrow().clone();
        crate::fluxos::call("triggers.save", json!({"id": t.id, "flow_id": fid, "trigger": t.config}), move |r| {
            let Some(p) = w.upgrade() else { return };
            match r {
                Ok(_) => {
                    p.issues.borrow_mut().remove(&key);
                    if let Some(x) = p.triggers.borrow_mut().iter_mut().find(|x| x.id == t.id) {
                        x.saved = true;
                    }
                }
                Err(e) => {
                    p.issues.borrow_mut().insert(key, e);
                }
            }
            p.canvas.set_data(p.canvas_data());
            p.refresh_header();
        });
    }

    /// Checks the flow a moment after the last change, then saves it when nothing is wrong.
    pub fn schedule_save(self: &Rc<Self>) {
        if let Some(s) = self.save_timer.borrow_mut().take() {
            s.remove();
        }
        let w = Rc::downgrade(self);
        let s = glib::timeout_add_local_once(Duration::from_millis(700), move || {
            if let Some(p) = w.upgrade() {
                *p.save_timer.borrow_mut() = None;
                p.save_now();
            }
        });
        *self.save_timer.borrow_mut() = Some(s);
    }

    fn save_now(self: &Rc<Self>) {
        let id = self.flow_id.borrow().clone();
        if id.is_empty() {
            return;
        }
        let doc = self.doc.borrow().clone();
        save_names(&id, &doc);
        let flow = doc.to_json();
        let name = self.name.borrow().clone();
        let w = Rc::downgrade(self);
        crate::fluxos::call("flows.validate", json!({"flow": flow}), move |r| {
            let Some(p) = w.upgrade() else { return };
            let issues: Vec<(String, String)> = match r {
                Ok(v) => v.as_array().into_iter().flatten().map(|i| (i["node"].as_str().unwrap_or("").to_string(), i["message"].as_str().unwrap_or("").to_string())).collect(),
                Err(e) => vec![(String::new(), e)],
            };
            {
                let mut map = p.issues.borrow_mut();
                map.retain(|k, _| k.starts_with(m::TRIGGER_PREFIX));
                for (k, v) in issues.iter() {
                    map.entry(k.clone()).and_modify(|x| {
                        *x += "; ";
                        x.push_str(v);
                    }).or_insert_with(|| v.clone());
                }
            }
            let changed_marks = !issues.is_empty();
            if issues.is_empty() {
                let w = Rc::downgrade(&p);
                crate::fluxos::call("flows.save", json!({"id": id, "name": name, "flow": flow}), move |r| {
                    if let Some(p) = w.upgrade() {
                        if let Err(e) = r {
                            // (the service names a block's problems by its id: a Subflow's mapping)
                            let ids: Vec<String> = p.doc.borrow().nodes.iter().map(|n| n.id.clone()).collect();
                            p.issues.borrow_mut().extend(m::split_issues(&e, &ids));
                        }
                        p.canvas.set_data(p.canvas_data());
                        p.refresh_header();
                        p.show_issues();
                        if p.selected.borrow().is_none() {
                            p.refresh_inspector();
                        }
                    }
                });
            }
            p.canvas.set_data(p.canvas_data());
            p.refresh_header();
            p.show_issues();
            // (the inspector shows the chosen block's problem; with nothing chosen, the list)
            if changed_marks && p.selected.borrow().is_none() {
                p.refresh_inspector();
            }
        });
    }

    /// The chosen block's problem line and the mapping rows show the problems as they are now.
    fn show_issues(&self) {
        if let Some((id, l)) = self.issue_label.borrow().as_ref() {
            let issue = self.issues.borrow().get(id).cloned();
            l.set_text(&m::issue_text(issue.as_deref().unwrap_or("")));
            l.set_visible(issue.is_some());
        }
        let f = self.on_issues.borrow().clone();
        if let Some(f) = f {
            f();
        }
    }

    pub fn remove_selected(self: &Rc<Self>) {
        let Some(sel) = self.selected.borrow_mut().take() else { return };
        if let Some(tid) = sel.strip_prefix(m::TRIGGER_PREFIX) {
            let tid = tid.to_string();
            self.triggers.borrow_mut().retain(|t| t.id != tid);
            self.issues.borrow_mut().remove(&sel);
            crate::fluxos::call("triggers.delete", json!({"id": tid}), |_| {});
            self.canvas.set_data(self.canvas_data());
            self.refresh_header();
        } else {
            self.doc.borrow_mut().remove_node(&sel);
            self.issues.borrow_mut().remove(&sel);
            self.changed();
        }
        self.refresh_inspector();
    }

    pub fn duplicate_selected(self: &Rc<Self>) {
        let Some(sel) = self.selected.borrow().clone() else { return };
        let Some(n) = self.doc.borrow().node(&sel).cloned() else { return };
        let rects = self.canvas.rects();
        let after = rects.iter().find(|r| r.0 == sel).map(|r| r.1);
        let (x, y) = m::free_spot(&rects, after, m::node_width(&n.kind), after.map(|r| r.h).unwrap_or(90.0));
        let id = self.doc.borrow().new_id(&n.kind);
        self.doc.borrow_mut().nodes.push(NodeDoc { id: id.clone(), x, y, ..n });
        *self.selected.borrow_mut() = Some(id.clone());
        self.changed();
        self.refresh_inspector();
        self.canvas.reveal(&id);
    }

    /// Below a decision's threshold the person is asked: an approval block with the decision's question, on the
    /// "pouca_confianca" way (the one already there is reused).
    pub fn ask_below_threshold(self: &Rc<Self>, src: &str) {
        let existing = {
            let d = self.doc.borrow();
            d.edges.iter().find(|e| e.source == src && e.handle.as_deref() == Some(m::LOW_CONFIDENCE)).map(|e| e.target.clone()).filter(|t| d.node(t).is_some_and(|n| n.kind == "approval"))
        };
        if existing.is_some() {
            return;
        }
        let rects = self.canvas.rects();
        let after = rects.iter().find(|r| r.0 == src).map(|r| r.1);
        let (x, y) = m::free_spot(&rects, after.map(|a| Rect { y: a.y + a.h + 40.0, ..a }), 172.0, 90.0);
        let q = {
            let d = self.doc.borrow();
            d.node(src).map(|n| n.data["questions"][n.data["route"].as_str().unwrap_or("")]["instructions"].as_str().unwrap_or("").to_string()).unwrap_or_default()
        };
        let id = self.doc.borrow().new_id("approval");
        let data = json!({"question": if q.is_empty() { tr("What should be done?").to_string() } else { q }, "details": tr("The decision model was not sure: you choose."), "timeout_s": 7200});
        self.edit_flow(|f| {
            f.nodes.push(NodeDoc { id: id.clone(), x, y, kind: "approval".into(), data, title: String::new() });
            f.set_way(src, Some(m::LOW_CONFIDENCE), Some(&id));
        });
    }

    // ── running ─────────────────────────────────────────────────────────────────────────────────────────────────

    fn build_test_popover(self: &Rc<Self>) {
        let pop = gtk::Popover::new();
        pop.add_css_class("fluxo-pop");
        self.test_btn.set_popover(Some(&pop));
        // (built each time it opens: the form follows the flow's input fields)
        let w = Rc::downgrade(self);
        pop.connect_show(move |pop| {
            if let Some(p) = w.upgrade() {
                p.fill_test_popover(pop);
            }
        });
    }

    /// "Testar com dados": a form made of the flow's input fields (design "Entrada do fluxo"), or, without fields,
    /// the free text the first block receives.
    fn fill_test_popover(self: &Rc<Self>, pop: &gtk::Popover) {
        let fields = self.doc.borrow().inputs();
        let c = caixa(Orientation::Vertical, 8, "fluxo-pop-corpo");
        c.set_size_request(320, -1);
        let t = rotulo(if fields.is_empty() { tr("Run with this input") } else { tr("Test with data") }, "fluxo-chave-titulo");
        t.set_xalign(0.0);
        c.append(&t);
        let s = rotulo(
            if fields.is_empty() { tr("What the first block receives, as if the trigger had fired (text or JSON).") } else { tr("The flow's input fields: each one becomes a variable of the run.") },
            "fluxo-chave-sub",
        );
        s.set_xalign(0.0);
        s.set_wrap(true);
        s.set_max_width_chars(40);
        c.append(&s);
        // the values: one box per field, or the free text
        let boxes: Rc<RefCell<Vec<(String, gtk::Entry)>>> = Rc::default();
        let tv = gtk::TextView::new();
        if fields.is_empty() {
            tv.add_css_class("fluxo-area-texto");
            tv.add_css_class("mono");
            tv.set_wrap_mode(gtk::WrapMode::WordChar);
            tv.buffer().set_text(self.test_values.borrow().get("").map(String::as_str).unwrap_or(""));
            let a = caixa(Orientation::Vertical, 0, "fluxo-area");
            a.append(&tv);
            c.append(&a);
        } else {
            let values = self.test_values.borrow();
            for f in &fields {
                let name = f["name"].as_str().unwrap_or("").to_string();
                let kind = f["kind"].as_str().unwrap_or("text");
                let row = caixa(Orientation::Horizontal, 6, "fluxo-form-cab");
                let n = rotulo(&name, "fluxo-campo-nome");
                n.set_xalign(0.0);
                row.append(&n);
                row.append(&rotulo(m::field_kind_name(kind), "fluxo-saida-chip"));
                if f["required"].as_bool().unwrap_or(false) {
                    row.append(&rotulo(tr("required"), "fluxo-obrigatorio"));
                }
                c.append(&row);
                let e = gtk::Entry::new();
                e.add_css_class("fluxo-campo");
                if matches!(kind, "list" | "object") {
                    e.add_css_class("mono");
                }
                e.set_placeholder_text(Some(f["description"].as_str().filter(|d| !d.is_empty()).unwrap_or(m::field_hint(kind))));
                e.set_text(values.get(&name).map(String::as_str).unwrap_or(""));
                c.append(&e);
                boxes.borrow_mut().push((name, e));
            }
        }
        // the service's refusal, as it came
        let err = rotulo("", "fluxo-problema");
        err.set_xalign(0.0);
        err.set_wrap(true);
        err.set_max_width_chars(40);
        err.set_visible(false);
        c.append(&err);
        let go = gtk::Button::with_label(tr("Run with this data"));
        go.add_css_class("fluxo-botao");
        go.add_css_class("primario");
        go.set_halign(Align::End);
        go.set_margin_top(4);
        c.append(&go);
        pop.set_child(Some(&c));
        let (w, pw) = (Rc::downgrade(self), pop.downgrade());
        go.connect_clicked(move |go| {
            let Some(p) = w.upgrade() else { return };
            let input = if boxes.borrow().is_empty() {
                let b = tv.buffer();
                let text = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
                p.test_values.borrow_mut().insert(String::new(), text.clone());
                text
            } else {
                let values: Vec<(String, String)> = boxes.borrow().iter().map(|(n, e)| (n.clone(), e.text().to_string())).collect();
                p.test_values.borrow_mut().extend(values.iter().cloned());
                m::form_object(&values).to_string()
            };
            go.set_sensitive(false);
            let (pw, err, go) = (pw.clone(), err.clone(), go.clone());
            p.run_then(&input, move |refused| {
                go.set_sensitive(true);
                match refused {
                    None => {
                        if let Some(pop) = pw.upgrade() {
                            pop.popdown();
                        }
                    }
                    Some(e) => {
                        err.set_text(&e);
                        err.set_visible(true);
                    }
                }
            });
        });
    }

    /// The flow opened last is loaded (its document and runs came).
    pub fn is_loaded(&self) -> bool {
        self.pending.get() == 0
    }

    /// Runs the flow now (`flows.run`) and follows it live.
    pub fn run(self: &Rc<Self>, input: &str) {
        self.run_then(input, |_| {});
    }

    /// Runs the flow; `done` gets the service's refusal (an input that does not fit the fields: "entrada inválida:
    /// …"), or None when the run started.
    fn run_then(self: &Rc<Self>, input: &str, done: impl FnOnce(Option<String>) + 'static) {
        let id = self.flow_id.borrow().clone();
        let w = Rc::downgrade(self);
        self.run_btn.set_sensitive(false);
        crate::fluxos::call("flows.run", json!({"id": id, "input": input}), move |r| {
            let Some(p) = w.upgrade() else { return };
            p.run_btn.set_sensitive(true);
            match r {
                Ok(v) => {
                    *p.run_refusal.borrow_mut() = None;
                    done(None);
                    let run = v["run_id"].as_str().unwrap_or("").to_string();
                    crate::fluxo_execucoes::follow(&p, &run);
                }
                Err(e) if e.starts_with(INVALID_INPUT) => {
                    *p.run_refusal.borrow_mut() = Some(e.clone());
                    done(Some(e));
                    p.refresh_inspector();
                }
                Err(e) => {
                    done(Some(e.clone()));
                    p.issues.borrow_mut().insert(String::new(), e);
                    p.refresh_header();
                    p.refresh_inspector();
                }
            }
        });
    }

    /// The window is narrow (design "Janelas estreitas"): the palette folds and the inspector gives way.
    pub fn set_narrow(self: &Rc<Self>, narrow: bool) {
        if self.narrow.get() == narrow {
            return;
        }
        self.narrow.set(narrow);
        crate::fluxo_execucoes::apply_narrow(self, narrow);
    }

    pub(crate) fn palette_widget(&self) -> &gtk::Box {
        &self.palette
    }

    /// The top bar: the builder's (not in Execuções, which has its own header; not in a narrow window, where the
    /// center's bar names the flow).
    pub(crate) fn update_top(&self) {
        self.top.set_visible(!self.is_runs_mode() && !self.narrow.get());
        let _ = &self.header_actions;
    }

    pub(crate) fn go_to(&self, page: &str) {
        let f = self.go.borrow().clone();
        if let Some(f) = f {
            f(page);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_states_read_as_the_screens_name_them() {
        assert_eq!(run_status_text("done").1, "ok");
        assert_eq!(run_status_text("waiting_approval").1, "aguarda");
        assert_eq!(run_status_text("denied").1, "falhou");
        assert_eq!(run_status_text("pending").1, "rodando");
    }

    #[test]
    fn trigger_ids_are_new_each_time() {
        let a = new_trigger_id();
        std::thread::sleep(std::time::Duration::from_millis(1));
        assert_ne!(a, new_trigger_id());
        assert!(a.starts_with('g'));
    }
}
