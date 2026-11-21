//! A flow's runs (design "Execuções · rastro", "Execução · decisões e probabilidades", "Execução ao vivo ·
//! aprovação"). Execuções: the numbers of the flow's runs, the list (filtered: all, done, with approval, failed) and,
//! on the right, the chosen run's trace — each block with its time and what it did, a decision with its options'
//! probabilities and its confidence against the threshold, what waits for the person — and the chosen step's output,
//! input and instructions. The live run: the builder's canvas marks each step as the service reports it (`run.step`,
//! `run.waiting`, `run.decided`, `run.status`) and the inspector becomes "Sua decisão" (allow once, always in this
//! flow, refuse: `runs.decide`; an Ask block takes an answer: `runs.answer`) and the live log; "Parar execução" is
//! `runs.cancel`.

use crate::conversa::ui_icon;
use crate::fluxo_construtor::{run_status_text, when_text, FlowPage};
use crate::fluxo_modelo::{self as m, FlowDoc, Step};
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

/// What waits for the person in a live run.
#[derive(Clone, Debug, PartialEq)]
pub struct Waiting {
    pub approval_id: String,
    pub node: String,
    pub kind: String,
    pub question: String,
    pub details: String,
    pub expires_at: i64,
}

/// The run followed live on the canvas.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Live {
    pub run_id: String,
    pub started: i64,
    pub status: String,
    pub steps: Vec<Step>,
    pub waiting: Option<Waiting>,
    /// ended, its marks still on the canvas until the flow changes
    pub finished_shown: bool,
}

impl Live {
    pub fn active(&self) -> bool {
        matches!(self.status.as_str(), "" | "pending" | "running" | "waiting_approval" | "waiting_input")
    }
}

fn now() -> i64 {
    glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0)
}

fn hms(t: i64) -> String {
    glib::DateTime::from_unix_local(t).ok().and_then(|d| d.format("%H:%M:%S").ok()).map(|s| s.to_string()).unwrap_or_default()
}

/// "1 h 56 min", "4 min", "30 s".
pub fn left_text(secs: i64) -> String {
    let s = secs.max(0);
    if s >= 3600 {
        format!("{} h {} min", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{} min", s / 60)
    } else {
        format!("{s} s")
    }
}

/// The words of a step in the trace and the live log: (what it is, what it did).
pub fn step_line(flow: &FlowDoc, s: &Step) -> String {
    let n = flow.node(&s.node);
    let clip = |t: &str| -> String {
        let t = t.trim().replace('\n', " ");
        if t.chars().count() > 140 { format!("{}…", t.chars().take(140).collect::<String>()) } else { t }
    };
    let way = m::way_taken(s).map(|w| {
        let l = m::way_label(n.map(|n| n.kind.as_str()).unwrap_or(""), Some(&w));
        if l.is_empty() { w } else { l }
    });
    match s.kind.as_str() {
        "condition" => {
            let rule = n.and_then(|n| n.data["conditions"].get(0)).map(|c| m::rule_text(&c["data"])).unwrap_or_default();
            format!("{rule} → {}", way.unwrap_or_default()).trim().to_string()
        }
        "approval" => match s.detail.as_str() {
            "allow_once" | "allow_always" => tr("allowed by you").to_string(),
            "expired" => tr("expired without an answer").to_string(),
            _ => tr("refused by you").to_string(),
        },
        "decide" => format!("{} · {} · {}", m::decision_model_name(n.map(|n| n.data["model"].as_str().unwrap_or("")).unwrap_or("")), tr("local"), tr("1 pass · 0 tokens generated")),
        _ if s.status == "failed" => clip(&s.detail),
        _ if !s.detail.is_empty() => clip(&s.detail),
        _ => clip(&s.output),
    }
}

// ── following a run live ────────────────────────────────────────────────────────────────────────────────────────

/// The service's run events for the page: the live run's steps, what waits, its end.
pub fn listen(page: &Rc<FlowPage>) {
    let w = Rc::downgrade(page);
    crate::fluxos::subscribe(&["run."], move |ev| {
        let Some(p) = w.upgrade() else { return false };
        let topic = ev["topic"].as_str().unwrap_or("");
        let d = &ev["data"];
        let run = d["run_id"].as_str().unwrap_or("").to_string();
        let ours = p.live.borrow().as_ref().is_some_and(|l| l.run_id == run);
        if !ours {
            // a run of this flow started elsewhere (its trigger): the list follows
            if topic == "run.status" && p.is_runs_mode() {
                p.reload_runs();
            }
            return true;
        }
        match topic {
            "run.step" => refetch(&p, &run),
            "run.waiting" => {
                if let Some(l) = p.live.borrow_mut().as_mut() {
                    l.waiting = Some(Waiting {
                        approval_id: d["approval_id"].as_str().unwrap_or("").into(),
                        node: d["node"].as_str().unwrap_or("").into(),
                        kind: d["kind"].as_str().unwrap_or("approval").into(),
                        question: d["question"].as_str().unwrap_or("").into(),
                        details: d["details"].as_str().unwrap_or("").into(),
                        expires_at: now() + d["expires_in_s"].as_i64().unwrap_or(7200),
                    });
                    l.status = if d["kind"] == "ask" { "waiting_input".into() } else { "waiting_approval".into() };
                }
                show_live(&p);
            }
            "run.decided" => {
                if let Some(l) = p.live.borrow_mut().as_mut() {
                    l.waiting = None;
                    l.status = "running".into();
                }
                show_live(&p);
            }
            "run.status" => {
                let st = d["status"].as_str().unwrap_or("").to_string();
                if let Some(l) = p.live.borrow_mut().as_mut() {
                    if !st.starts_with("waiting") {
                        l.waiting = None;
                    }
                    l.status = st.clone();
                    if !l.active() {
                        l.finished_shown = true;
                    }
                }
                refetch(&p, &run);
                if !p.live.borrow().as_ref().is_some_and(|l| l.active()) {
                    p.reload_runs();
                }
            }
            _ => {}
        }
        true
    });
    // the header's running time
    let w: Weak<FlowPage> = Rc::downgrade(page);
    glib::timeout_add_local(Duration::from_secs(1), move || {
        let Some(p) = w.upgrade() else { return glib::ControlFlow::Break };
        if p.live.borrow().as_ref().is_some_and(|l| l.active()) && p.root.is_mapped() {
            p.refresh_header();
        }
        glib::ControlFlow::Continue
    });
}

/// Follows a run on the builder's canvas (just started, or one waiting for the person).
pub fn follow(page: &Rc<FlowPage>, run: &str) {
    *page.live.borrow_mut() = Some(Live { run_id: run.to_string(), started: now(), status: "pending".into(), ..Default::default() });
    page.show_mode("editar");
    *page.selected.borrow_mut() = None;
    refetch(page, run);
    // what already waits (a run opened from Execuções)
    let (w, run2) = (Rc::downgrade(page), run.to_string());
    crate::fluxos::call("approvals.open", json!({}), move |r| {
        let Some(p) = w.upgrade() else { return };
        if let Some(a) = r.unwrap_or_default().as_array().into_iter().flatten().find(|a| a["run_id"] == json!(run2)) {
            if let Some(l) = p.live.borrow_mut().as_mut().filter(|l| l.run_id == run2) {
                l.waiting = Some(Waiting {
                    approval_id: a["id"].as_str().unwrap_or("").into(),
                    node: a["node_id"].as_str().unwrap_or("").into(),
                    kind: if p.doc.borrow().node(a["node_id"].as_str().unwrap_or("")).is_some_and(|n| n.kind == "ask") { "ask".into() } else { "approval".into() },
                    question: a["question"].as_str().unwrap_or("").into(),
                    details: a["details"].as_str().unwrap_or("").into(),
                    expires_at: a["expires_at"].as_i64().unwrap_or(0),
                });
                l.status = "waiting_approval".into();
            }
            show_live(&p);
        }
    });
    show_live(page);
}

fn refetch(page: &Rc<FlowPage>, run: &str) {
    let (w, run) = (Rc::downgrade(page), run.to_string());
    crate::fluxos::call("runs.get", json!({"id": run}), move |r| {
        let (Some(p), Ok(v)) = (w.upgrade(), r) else { return };
        let mut ended = false;
        if let Some(l) = p.live.borrow_mut().as_mut().filter(|l| l.run_id == run) {
            l.steps = v["steps"].as_array().into_iter().flatten().map(Step::from_json).collect();
            l.started = v["run"]["created_at"].as_i64().unwrap_or(l.started);
            let st = v["run"]["status"].as_str().unwrap_or("").to_string();
            // (the service's word is final once the run ended)
            if !st.is_empty() && !matches!(st.as_str(), "pending" | "running") {
                l.status = st;
                if !l.active() {
                    l.waiting = None;
                    l.finished_shown = true;
                    ended = true;
                }
            } else if l.status.is_empty() || l.status == "pending" {
                l.status = st;
            }
        }
        show_live(&p);
        // (a run that ended before its events arrived: the header and Execuções learn of it here)
        if ended {
            p.reload_runs();
        }
    });
}

fn show_live(page: &Rc<FlowPage>) {
    let live = page.live.borrow().clone();
    let banner = live.as_ref().and_then(|l| l.waiting.clone());
    page.canvas.banner.set_visible(banner.is_some());
    page.canvas.banner_note.set_visible(!page.narrow.get());
    page.canvas.tip.set_visible(banner.is_none());
    if let Some(w) = banner {
        page.canvas.banner_text.set_text(&format!("{}: {}", tr("Waiting for you"), w.question));
    }
    page.refresh_header();
    page.canvas.set_data(page.canvas_data());
    page.refresh_inspector();
    if let Some(w) = live.as_ref().and_then(|l| l.waiting.as_ref()) {
        page.canvas.reveal(&w.node);
    }
}

/// The inspector while a run is live: "Sua decisão" when something waits, the live log, "Parar execução".
pub fn live_panel(page: &Rc<FlowPage>) {
    let c = &page.insp_content;
    while let Some(x) = c.first_child() {
        c.remove(&x);
    }
    let f = &page.insp_footer;
    while let Some(x) = f.first_child() {
        f.remove(&x);
    }
    let Some(l) = page.live.borrow().clone() else { return };
    let doc = page.doc.borrow().clone();
    if let Some(wt) = &l.waiting {
        let cab = caixa(Orientation::Horizontal, 8, "");
        cab.append(&ui_icon("escudo", "#A5A1E2", 14));
        cab.append(&rotulo(if wt.kind == "ask" { tr("YOUR ANSWER") } else { tr("YOUR DECISION") }, "fluxo-secao-cab"));
        c.append(&cab);
        let card = caixa(Orientation::Vertical, 8, "fluxo-decisao");
        let q = rotulo(&wt.question, "fluxo-decisao-titulo");
        q.set_xalign(0.0);
        q.set_wrap(true);
        q.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        card.append(&q);
        if !wt.details.is_empty() {
            let d = rotulo(&wt.details, "fluxo-chave-sub");
            d.set_xalign(0.0);
            d.set_wrap(true);
            d.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            card.append(&d);
        }
        // what will be done after it (the next block's action, as the design shows it)
        let next = doc.edges.iter().find(|e| e.source == wt.node && e.handle.as_deref() == Some("sim")).and_then(|e| doc.node(&e.target)).filter(|n| n.kind == "system_action");
        if let Some(n) = next {
            let mut lines = vec![n.data["tool"].as_str().unwrap_or("").to_string()];
            if let Some(a) = n.data["arguments"].as_object() {
                for (k, v) in a.iter().take(4) {
                    lines.push(format!("{k}: {}", if let Some(s) = v.as_str() { format!("\"{s}\"") } else { v.to_string() }));
                }
            }
            let b = caixa(Orientation::Vertical, 4, "fluxo-mono-caixa");
            for t in lines {
                let x = rotulo(&t, "fluxo-mono");
                x.set_xalign(0.0);
                x.set_wrap(true);
                x.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                b.append(&x);
            }
            card.append(&b);
        }
        let meta = caixa(Orientation::Horizontal, 6, "");
        // (the two texts give way in a narrow panel instead of widening it)
        let lvl = rotulo(&format!("{} · {}", tr("user level"), tr("undoable")), "fluxo-chave-sub");
        lvl.set_ellipsize(gtk::pango::EllipsizeMode::End);
        meta.append(&lvl);
        let exp = rotulo(&format!("· {} {}", tr("expires in"), left_text(wt.expires_at - now())), "fluxo-chave-sub");
        exp.add_css_class("atencao");
        exp.set_ellipsize(gtk::pango::EllipsizeMode::End);
        meta.append(&exp);
        card.append(&meta);
        let aid = wt.approval_id.clone();
        if wt.kind == "ask" {
            let e = gtk::Entry::new();
            e.add_css_class("fluxo-campo");
            e.set_placeholder_text(Some(tr("Your answer")));
            card.append(&e);
            let send = gtk::Button::with_label(tr("Answer"));
            send.add_css_class("fluxo-botao");
            send.add_css_class("primario");
            let (w, aid2, e2) = (Rc::downgrade(page), aid.clone(), e.clone());
            let go = move || {
                let t = e2.text().trim().to_string();
                if t.is_empty() {
                    return;
                }
                if let Some(p) = w.upgrade() {
                    decide(&p, "runs.answer", json!({"approval_id": aid2, "text": t}));
                }
            };
            let g2 = go.clone();
            e.connect_activate(move |_| g2());
            send.connect_clicked(move |_| go());
            card.append(&send);
        } else {
            let allow = gtk::Button::new();
            allow.add_css_class("fluxo-botao");
            allow.add_css_class("primario");
            let ac = caixa(Orientation::Horizontal, 8, "");
            ac.set_halign(Align::Center);
            ac.append(&ui_icon("check", "#1C1D24", 14));
            ac.append(&rotulo(tr("Allow once"), ""));
            allow.set_child(Some(&ac));
            card.append(&allow);
            let row = caixa(Orientation::Horizontal, 8, "");
            row.set_homogeneous(true);
            let always = gtk::Button::with_label(tr("Always in this flow"));
            always.add_css_class("fluxo-botao");
            let deny = gtk::Button::with_label(tr("Refuse"));
            deny.add_css_class("fluxo-botao");
            deny.add_css_class("perigo");
            row.append(&always);
            row.append(&deny);
            card.append(&row);
            for (b, d) in [(&allow, "allow_once"), (&always, "allow_always"), (&deny, "deny")] {
                let (w, aid) = (Rc::downgrade(page), aid.clone());
                b.connect_clicked(move |_| {
                    if let Some(p) = w.upgrade() {
                        decide(&p, "runs.decide", json!({"approval_id": aid, "decision": d}));
                    }
                });
            }
        }
        c.append(&card);
        let nota = caixa(Orientation::Horizontal, 8, "fluxo-nota-solta");
        let ic = ui_icon("info", "#8E8F99", 14);
        ic.set_valign(Align::Start);
        nota.append(&ic);
        let nt = rotulo(tr("System or full level actions do not show here: KalangoOS asks on the approval screen, which no agent can imitate."), "fluxo-chave-sub");
        nt.set_xalign(0.0);
        nt.set_wrap(true);
        nt.set_max_width_chars(36);
        nota.append(&nt);
        c.append(&nota);
    } else {
        let cab = caixa(Orientation::Horizontal, 8, "");
        cab.append(&ui_icon("raio", "#A5A1E2", 14));
        let (st, _) = run_status_text(&l.status);
        cab.append(&rotulo(&format!("{} · {}", tr("RUN"), st.to_uppercase()), "fluxo-secao-cab"));
        c.append(&cab);
    }
    // the live log
    let t = rotulo(tr("LIVE"), "fluxo-secao");
    t.set_xalign(0.0);
    c.append(&t);
    let log = caixa(Orientation::Vertical, 4, "fluxo-mono-caixa");
    log.add_css_class("ao-vivo");
    for s in &l.steps {
        let row = caixa(Orientation::Horizontal, 10, "");
        let class = if s.status == "failed" { "falhou" } else if s.kind == "system_action" && s.status == "done" { "ok" } else { "" };
        for (txt, w) in [(hms(s.started), 8), (crate::fluxo_inspetor::kind_word(&s.kind), 9)] {
            let x = rotulo(&txt, "fluxo-mono");
            x.set_xalign(0.0);
            x.set_width_chars(w);
            if !class.is_empty() {
                x.add_css_class(class);
            }
            row.append(&x);
        }
        let mut what = page.node_title(&s.node);
        if s.kind == "condition" || s.kind == "decide" {
            if let Some(wy) = m::way_taken(s) {
                what = m::way_label(&s.kind, Some(&wy));
                if what.is_empty() {
                    what = wy;
                }
            }
        }
        if s.status == "done" && s.kind == "system_action" {
            what += " ✓";
        }
        if s.status == "failed" {
            what += " ✕";
        }
        let x = rotulo(&what, "fluxo-mono");
        x.set_xalign(0.0);
        x.set_ellipsize(gtk::pango::EllipsizeMode::End);
        x.set_hexpand(true);
        if !class.is_empty() {
            x.add_css_class(class);
        }
        row.append(&x);
        log.append(&row);
    }
    if let Some(wt) = &l.waiting {
        let row = caixa(Orientation::Horizontal, 10, "");
        for (txt, w) in [(hms(now()), 8), (tr("waiting").to_string(), 9), (format!("{}…", if wt.kind == "ask" { tr("answer") } else { tr("approval") }), 0)] {
            let x = rotulo(&txt, "fluxo-mono");
            x.set_xalign(0.0);
            if w > 0 {
                x.set_width_chars(w);
            }
            x.add_css_class("aguarda");
            row.append(&x);
        }
        log.append(&row);
    }
    if l.steps.is_empty() && l.waiting.is_none() {
        let x = rotulo(tr("starting…"), "fluxo-mono");
        x.set_xalign(0.0);
        log.append(&x);
    }
    c.append(&log);
    // the footer: stop it (or, once it ended, back to editing)
    if l.active() {
        let stop = gtk::Button::new();
        stop.add_css_class("fluxo-botao");
        stop.add_css_class("perigo");
        stop.set_hexpand(true);
        let sc = caixa(Orientation::Horizontal, 8, "");
        sc.set_halign(Align::Center);
        sc.append(&ui_icon("quadrado", "#EFA2A6", 14));
        sc.append(&rotulo(tr("Stop the run"), ""));
        stop.set_child(Some(&sc));
        let (w, run) = (Rc::downgrade(page), l.run_id.clone());
        stop.connect_clicked(move |_| {
            let w = w.clone();
            crate::fluxos::call("runs.cancel", json!({"id": run}), move |_| {
                if let Some(p) = w.upgrade() {
                    if let Some(l) = p.live.borrow_mut().as_mut() {
                        l.status = "stopped".into();
                        l.waiting = None;
                        l.finished_shown = true;
                    }
                    show_live(&p);
                    p.reload_runs();
                }
            });
        });
        f.append(&stop);
    }
}

fn decide(page: &Rc<FlowPage>, method: &str, params: Value) {
    let w = Rc::downgrade(page);
    crate::fluxos::call(method, params, move |r| {
        let Some(p) = w.upgrade() else { return };
        if let Some(l) = p.live.borrow_mut().as_mut() {
            l.waiting = None;
            if r.as_ref().is_ok_and(|v| v == &json!(false)) {
                // already decided or expired: the service says what happened next
                l.status = "running".into();
            } else {
                l.status = "running".into();
            }
        }
        show_live(&p);
        let run = p.live.borrow().as_ref().map(|l| l.run_id.clone()).unwrap_or_default();
        refetch(&p, &run);
    });
}

// ── Execuções ───────────────────────────────────────────────────────────────────────────────────────────────────

pub struct RunsView {
    pub root: gtk::Box,
    page: Weak<FlowPage>,
    title: gtk::Label,
    chip: gtk::Label,
    sub: gtk::Label,
    runs_button: gtk::Button,
    stats: [(gtk::Label, gtk::Label); 4],
    filter: RefCell<String>,
    table: gtk::Box,
    decisions: gtk::Box,
    trace_title: gtk::Label,
    trace_sub: gtk::Label,
    trace: gtk::Box,
    detail_title: gtk::Label,
    detail_tabs: gtk::Box,
    detail: gtk::Box,
    footer: gtk::Box,
    chosen: RefCell<Option<String>>,
    chosen_step: Cell<i64>,
    detail_tab: RefCell<String>,
    /// the chosen run's steps (fetched on choosing)
    steps: RefCell<Vec<Step>>,
    run: RefCell<Value>,
    approvals: RefCell<Vec<Value>>,
    pub side: gtk::Box,
}

impl RunsView {
    pub fn new(page: &Rc<FlowPage>) -> Rc<RunsView> {
        let root = caixa(Orientation::Horizontal, 0, "");
        root.set_vexpand(true);
        let body = caixa(Orientation::Vertical, 0, "central-corpo");
        body.add_css_class("fluxo-exec-corpo");
        body.set_hexpand(true);
        let top = caixa(Orientation::Horizontal, 12, "");
        let names = caixa(Orientation::Vertical, 4, "");
        names.set_hexpand(true);
        let l1 = caixa(Orientation::Horizontal, 10, "");
        let title = rotulo("", "central-titulo");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l1.append(&title);
        let chip = rotulo("", "fluxo-estado");
        chip.set_valign(Align::Center);
        l1.append(&chip);
        names.append(&l1);
        let sub = rotulo("", "central-sub");
        sub.set_xalign(0.0);
        sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
        names.append(&sub);
        top.append(&names);
        let seg = caixa(Orientation::Horizontal, 0, "fluxo-seg");
        seg.add_css_class("topo");
        seg.set_valign(Align::Start);
        let b_edit = gtk::Button::with_label(tr("Edit"));
        b_edit.add_css_class("fluxo-seg-item");
        let b_runs = gtk::Button::with_label(tr("Runs"));
        b_runs.add_css_class("fluxo-seg-item");
        b_runs.add_css_class("ativo");
        seg.append(&b_edit);
        seg.append(&b_runs);
        top.append(&seg);
        body.append(&top);
        // the numbers
        let cards = gtk::FlowBox::new();
        cards.set_selection_mode(gtk::SelectionMode::None);
        cards.set_homogeneous(true);
        cards.set_min_children_per_line(2);
        cards.set_max_children_per_line(4);
        cards.set_column_spacing(12);
        cards.set_row_spacing(12);
        cards.set_margin_top(18);
        let mk = |class: &str| {
            let c = caixa(Orientation::Vertical, 2, "fluxo-numero");
            let v = rotulo("—", "fluxo-numero-valor");
            if !class.is_empty() {
                v.add_css_class(class);
            }
            v.set_xalign(0.0);
            let n = rotulo("", "fluxo-chave-sub");
            n.set_xalign(0.0);
            c.append(&v);
            c.append(&n);
            cards.insert(&c, -1);
            (v, n)
        };
        let stats = [mk("ok"), mk("falhou"), mk(""), mk("")];
        body.append(&cards);
        let frow = caixa(Orientation::Horizontal, 8, "");
        frow.set_margin_top(18);
        let pw = Rc::downgrade(page);
        let filter_seg = crate::fluxo_inspetor::segmented(
            &[("todas", tr("All runs").into()), ("ok", tr("Completed runs").into()), ("aprovacao", tr("With approval").into()), ("falhas", tr("Failed runs").into())],
            "todas",
            move |f| {
                if let Some(rv) = pw.upgrade().and_then(|p| p.runs_view.borrow().clone()) {
                    *rv.filter.borrow_mut() = f.to_string();
                    rv.render_table();
                }
            },
        );
        frow.append(&filter_seg);
        let sp = caixa(Orientation::Horizontal, 0, "");
        sp.set_hexpand(true);
        frow.append(&sp);
        let kept = rotulo(tr("Kept for 90 days"), "fluxo-chave-sub");
        kept.set_valign(Align::Center);
        frow.append(&kept);
        body.append(&frow);
        let table = caixa(Orientation::Vertical, 0, "fluxo-tabela");
        table.set_margin_top(14);
        let decisions = caixa(Orientation::Vertical, 8, "");
        let inner = caixa(Orientation::Vertical, 0, "");
        inner.append(&table);
        inner.append(&decisions);
        let sc = gtk::ScrolledWindow::new();
        sc.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
        sc.set_vexpand(true);
        sc.set_child(Some(&inner));
        body.append(&sc);
        root.append(&body);
        // the trace, on the right
        let side = caixa(Orientation::Vertical, 0, "fluxo-insp");
        side.add_css_class("rastro");
        side.set_size_request(500, -1);
        side.set_hexpand(false);
        let sc_in = caixa(Orientation::Vertical, 0, "fluxo-insp-conteudo");
        let cab = caixa(Orientation::Horizontal, 8, "");
        cab.append(&ui_icon("lista", "#A5A1E2", 14));
        let trace_title = rotulo("", "fluxo-secao-cab");
        trace_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        trace_title.set_hexpand(true);
        trace_title.set_xalign(0.0);
        cab.append(&trace_title);
        sc_in.append(&cab);
        let trace_sub = rotulo("", "fluxo-chave-sub");
        trace_sub.set_xalign(0.0);
        trace_sub.set_use_markup(true);
        trace_sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
        sc_in.append(&trace_sub);
        let trace = caixa(Orientation::Vertical, 4, "fluxo-rastro");
        sc_in.append(&trace);
        let detail_title = rotulo("", "fluxo-secao");
        detail_title.set_xalign(0.0);
        detail_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        sc_in.append(&detail_title);
        let dbox = caixa(Orientation::Vertical, 8, "fluxo-passo");
        let detail_tabs = caixa(Orientation::Horizontal, 0, "");
        dbox.append(&detail_tabs);
        let detail = caixa(Orientation::Vertical, 0, "");
        dbox.append(&detail);
        sc_in.append(&dbox);
        let ss = gtk::ScrolledWindow::new();
        ss.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        ss.set_vexpand(true);
        ss.set_child(Some(&sc_in));
        side.append(&ss);
        let footer = caixa(Orientation::Horizontal, 8, "fluxo-insp-rodape");
        footer.set_homogeneous(true);
        side.append(&footer);
        root.append(&side);
        let rv = Rc::new(RunsView {
            root, page: Rc::downgrade(page), title, chip, sub, runs_button: b_runs.clone(), stats, filter: RefCell::new("todas".into()), table, decisions, trace_title, trace_sub, trace, detail_title,
            detail_tabs, detail, footer, chosen: RefCell::default(), chosen_step: Cell::new(-1), detail_tab: RefCell::new("saida".into()), steps: RefCell::default(),
            run: RefCell::default(), approvals: RefCell::default(), side,
        });
        let w = Rc::downgrade(page);
        b_edit.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.show_mode("editar");
            }
        });
        rv
    }

    /// The next render shows the newest run.
    pub fn reset_choice(&self) {
        *self.chosen.borrow_mut() = None;
    }

    fn runs(&self) -> Vec<Value> {
        self.page.upgrade().map(|p| p.runs.borrow().clone()).unwrap_or_default()
    }

    pub fn render(self: &Rc<Self>) {
        let Some(p) = self.page.upgrade() else { return };
        self.title.set_text(&p.name.borrow());
        self.chip.set_text(if p.enabled.get() { tr("active") } else { tr("paused") });
        if p.enabled.get() { self.chip.remove_css_class("pausado") } else { self.chip.add_css_class("pausado") }
        let runs = self.runs();
        self.runs_button.set_label(&format!("{} {}", tr("Runs"), runs.len()));
        let month = now() - 30 * 86_400;
        let recent: Vec<&Value> = runs.iter().filter(|r| r["created_at"].as_i64().unwrap_or(0) >= month).collect();
        let steps_of = |r: &Value| p.recent.borrow().iter().find(|(x, _)| x["id"] == r["id"]).map(|(_, s)| s.clone());
        let denied = recent.iter().filter(|r| refused(r, steps_of(r).as_deref())).count();
        let count = |s: &[&str]| recent.iter().filter(|r| s.contains(&r["status"].as_str().unwrap_or(""))).count();
        let (ok, failed) = (count(&["done"]) - recent.iter().filter(|r| r["status"] == "done" && refused(r, steps_of(r).as_deref())).count(), count(&["failed"]));
        let total = if recent.len() == 1 { tr("1 run in the last 30 days").to_string() } else { trf("{n} runs in the last 30 days", &[("n", recent.len().to_string())]) };
        let mut parts = vec![total, format!("{ok} ok")];
        if denied > 0 {
            parts.push(trf("{n} refused by you", &[("n", denied.to_string())]));
        }
        if failed > 0 {
            parts.push(trf("{n} failed", &[("n", failed.to_string())]));
        }
        self.sub.set_text(&parts.join(" · "));
        let durs: Vec<f64> = recent.iter().filter_map(|r| Some((r["finished_at"].as_i64()? - r["created_at"].as_i64()?) as f64)).collect();
        let avg = if durs.is_empty() { None } else { Some(durs.iter().sum::<f64>() / durs.len() as f64) };
        let tokens: i64 = recent.iter().map(|r| r["tokens"].as_i64().unwrap_or(0)).sum();
        let vals = [
            (ok.to_string(), if ok == 1 { tr("completed").to_string() } else { tr("completed (plural)").to_string() }),
            (failed.to_string(), if failed == 1 { tr("failed").to_string() } else { tr("failed (plural)").to_string() }),
            (avg.map(m::duration_text).unwrap_or_else(|| "—".into()), tr("average duration").to_string()),
            (m::tokens_text(tokens), tr("tokens · local").to_string()),
        ];
        for ((v, n), (a, b)) in self.stats.iter().zip(vals) {
            v.set_text(&a);
            n.set_text(&b);
        }
        self.render_table();
        self.render_decisions();
        let chosen = self.chosen.borrow().clone();
        let first = runs.first().and_then(|r| r["id"].as_str()).map(str::to_string);
        match chosen.filter(|c| runs.iter().any(|r| r["id"] == json!(c))).or(first) {
            Some(id) => self.choose(&id),
            None => {
                self.trace_title.set_text(tr("NO RUN YET"));
                self.trace_sub.set_text(tr("Run the flow now, or wait for its trigger."));
                for b in [&self.trace, &self.detail_tabs, &self.detail, &self.footer] {
                    while let Some(x) = b.first_child() {
                        b.remove(&x);
                    }
                }
                self.detail_title.set_text("");
            }
        }
    }

    fn render_table(self: &Rc<Self>) {
        while let Some(x) = self.table.first_child() {
            self.table.remove(&x);
        }
        let runs = self.runs();
        let p = self.page.upgrade();
        let f = self.filter.borrow().clone();
        let recent: Vec<(String, Vec<Step>)> = p.as_ref().map(|p| p.recent.borrow().iter().map(|(r, s)| (r["id"].as_str().unwrap_or("").to_string(), s.clone())).collect()).unwrap_or_default();
        let total = p.as_ref().map(|p| p.doc.borrow().nodes.len()).unwrap_or(0);
        let fields = p.as_ref().map(|p| p.doc.borrow().inputs()).unwrap_or_default();
        let has_approval = |id: &str| recent.iter().find(|(r, _)| r == id).is_some_and(|(_, s)| s.iter().any(|x| x.kind == "approval"));
        let shown: Vec<&Value> = runs
            .iter()
            .filter(|r| {
                let st = r["status"].as_str().unwrap_or("");
                match f.as_str() {
                    "ok" => st == "done",
                    "falhas" => matches!(st, "failed" | "stopped" | "denied"),
                    "aprovacao" => st.starts_with("waiting") || st == "denied" || has_approval(r["id"].as_str().unwrap_or("")),
                    _ => true,
                }
            })
            .collect();
        let grid = gtk::Grid::new();
        grid.set_column_spacing(0);
        let cols = [tr("When"), tr("Trigger"), tr("Steps"), tr("Duration"), tr("Tokens"), tr("Result")];
        let head = caixa(Orientation::Horizontal, 0, "fluxo-tabela-cab");
        let widths = [130, 0, 90, 80, 80, 150];
        let mk_row = |cells: Vec<gtk::Widget>, class: &str| {
            let r = caixa(Orientation::Horizontal, 0, class);
            for (i, w) in cells.into_iter().enumerate() {
                if widths[i] > 0 {
                    w.set_size_request(widths[i], -1);
                } else {
                    w.set_hexpand(true);
                }
                r.append(&w);
            }
            r
        };
        let lead = caixa(Orientation::Horizontal, 0, "");
        lead.set_size_request(22, -1);
        head.append(&lead);
        let hc: Vec<gtk::Widget> = cols
            .iter()
            .map(|c| {
                let l = rotulo(c, "fluxo-tabela-col");
                l.set_xalign(0.0);
                l.upcast()
            })
            .collect();
        head.append(&mk_row(hc, ""));
        self.table.append(&head);
        let chosen = self.chosen.borrow().clone();
        for r in shown {
            let id = r["id"].as_str().unwrap_or("").to_string();
            let steps = recent.iter().find(|(x, _)| *x == id).map(|(_, s)| s.clone());
            let st = if refused(r, steps.as_deref()) { "denied" } else { r["status"].as_str().unwrap_or("") };
            let (txt, class) = run_status_text(st);
            let input = steps.as_ref().and_then(|s| s.first()).map(|s| s.input.clone()).unwrap_or_default();
            let input: String = m::input_summary(&input, &fields).chars().take(60).collect();
            let n = steps.as_ref().map(|s| s.len());
            let b = gtk::Button::new();
            b.add_css_class("fluxo-tabela-linha");
            if chosen.as_deref() == Some(id.as_str()) {
                b.add_css_class("escolhida");
            }
            let row = caixa(Orientation::Horizontal, 0, "");
            let dot = caixa(Orientation::Horizontal, 0, "fluxo-no-ponto");
            dot.add_css_class(match class {
                "ok" => "feito",
                "aguarda" => "aguarda",
                "falhou" => "falhou",
                "rodando" => "rodando",
                _ => "neutro",
            });
            dot.set_size_request(6, 6);
            dot.set_valign(Align::Center);
            dot.set_halign(Align::Start);
            let lead = caixa(Orientation::Horizontal, 0, "");
            lead.set_size_request(22, -1);
            lead.append(&dot);
            row.append(&lead);
            let dur = r["finished_at"].as_i64().map(|f| m::duration_text((f - r["created_at"].as_i64().unwrap_or(f)) as f64)).unwrap_or_else(|| "—".into());
            let mut cells: Vec<gtk::Widget> = Vec::new();
            for (t, mono) in [
                (when_text(r["created_at"].as_i64().unwrap_or(0)), true),
                (if input.is_empty() { tr("by hand").to_string() } else { input }, false),
                (n.map(|n| trf("{n} of {total}", &[("n", n.to_string()), ("total", total.max(n).to_string())])).unwrap_or_else(|| "—".into()), false),
                (dur, true),
                (m::tokens_text(r["tokens"].as_i64().unwrap_or(0)), true),
            ] {
                let l = rotulo(&t, if mono { "fluxo-tabela-mono" } else { "fluxo-tabela-texto" });
                l.set_xalign(0.0);
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                l.set_margin_end(12);
                cells.push(l.upcast());
            }
            let rc = caixa(Orientation::Horizontal, 0, "");
            let chip = rotulo(&if st == "failed" { r["error_code"].as_str().map(|e| format!("{}: {e}", tr("failed"))).unwrap_or(txt) } else { txt }, "fluxo-resultado");
            chip.add_css_class(class);
            chip.set_ellipsize(gtk::pango::EllipsizeMode::End);
            rc.append(&chip);
            cells.push(rc.upcast());
            row.append(&mk_row(cells, ""));
            b.set_child(Some(&row));
            let (w, id2) = (Rc::downgrade(self), id.clone());
            b.connect_clicked(move |_| {
                if let Some(rv) = w.upgrade() {
                    rv.chosen_step.set(-1);
                    rv.choose(&id2);
                    rv.render_table();
                }
            });
            self.table.append(&b);
        }
        let _ = grid;
    }

    /// "Decisões desta rotina · 30 dias" (flows with a Decidir block).
    fn render_decisions(self: &Rc<Self>) {
        while let Some(x) = self.decisions.first_child() {
            self.decisions.remove(&x);
        }
        let Some(p) = self.page.upgrade() else { return };
        if !p.doc.borrow().nodes.iter().any(|n| n.kind == "decide") {
            return;
        }
        let runs: Vec<Vec<Step>> = p.recent.borrow().iter().map(|(_, s)| s.clone()).collect();
        let st = m::decision_stats(&runs);
        let cab = caixa(Orientation::Horizontal, 8, "");
        cab.set_margin_top(22);
        cab.append(&ui_icon("compartilhar", "#A5A1E2", 14));
        cab.append(&rotulo(tr("DECISIONS OF THIS ROUTINE · RECENT RUNS"), "fluxo-secao-cab"));
        self.decisions.append(&cab);
        let card = caixa(Orientation::Vertical, 8, "fluxo-mono-caixa");
        let total = (st.alone + st.asked).max(1);
        let rows = [
            (tr("Runs").to_string(), st.runs.to_string()),
            (tr("Decided alone").to_string(), format!("{} ({}%)", st.alone, st.alone * 100 / total)),
            (tr("Asked you").to_string(), format!("{} ({})", st.asked, tr("low confidence"))),
            (
                tr("Time per decision").to_string(),
                match (st.median_ms, st.p95_ms) {
                    (Some(a), Some(b)) => format!("{} {a} ms · p95 {b} ms", tr("median")),
                    _ => "—".into(),
                },
            ),
            (tr("From the cache").to_string(), trf("{n} (repeated inputs)", &[("n", st.cached.to_string())])),
        ];
        for (k, v) in rows {
            let r = caixa(Orientation::Horizontal, 8, "");
            let a = rotulo(&k, "fluxo-chave-sub");
            a.set_hexpand(true);
            a.set_xalign(0.0);
            r.append(&a);
            r.append(&rotulo(&v, "fluxo-valor-forte"));
            card.append(&r);
        }
        self.decisions.append(&card);
        let nota = caixa(Orientation::Horizontal, 8, "fluxo-nota-solta");
        let ic = ui_icon("info", "#8E8F99", 14);
        ic.set_valign(Align::Start);
        nota.append(&ic);
        let nt = rotulo(tr("The probabilities come calibrated from the model, but not for your data: adjust the threshold if it asks too much or too little."), "fluxo-chave-sub");
        nt.set_xalign(0.0);
        nt.set_wrap(true);
        nota.append(&nt);
        self.decisions.append(&nota);
        let b = gtk::Button::new();
        b.add_css_class("fluxo-botao");
        b.set_halign(Align::Start);
        let bc = caixa(Orientation::Horizontal, 8, "");
        bc.append(&ui_icon("ajustes", "#E2E2E6", 14));
        bc.append(&rotulo(tr("Adjust the threshold"), ""));
        b.set_child(Some(&bc));
        let w = Rc::downgrade(&p);
        b.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                let id = p.doc.borrow().nodes.iter().find(|n| n.kind == "decide").map(|n| n.id.clone());
                p.show_mode("editar");
                p.select(id.clone());
                if let Some(id) = id {
                    p.canvas.reveal(&id);
                }
            }
        });
        self.decisions.append(&b);
    }

    /// Shows a run's trace on the right.
    pub fn choose(self: &Rc<Self>, id: &str) {
        *self.chosen.borrow_mut() = Some(id.to_string());
        let (w, id2) = (Rc::downgrade(self), id.to_string());
        crate::fluxos::call("runs.get", json!({"id": id}), move |r| {
            let (Some(rv), Ok(v)) = (w.upgrade(), r) else { return };
            if rv.chosen.borrow().as_deref() != Some(id2.as_str()) {
                return;
            }
            *rv.steps.borrow_mut() = v["steps"].as_array().into_iter().flatten().map(Step::from_json).collect();
            *rv.run.borrow_mut() = v["run"].clone();
            let w = Rc::downgrade(&rv);
            crate::fluxos::call("approvals.open", json!({}), move |r| {
                if let Some(rv) = w.upgrade() {
                    let run = rv.chosen.borrow().clone().unwrap_or_default();
                    *rv.approvals.borrow_mut() = r.unwrap_or_default().as_array().into_iter().flatten().filter(|a| a["run_id"] == json!(run)).cloned().collect();
                    rv.render_trace();
                }
            });
        });
    }

    fn render_trace(self: &Rc<Self>) {
        let Some(p) = self.page.upgrade() else { return };
        let flow = p.doc.borrow().clone();
        let run = self.run.borrow().clone();
        let steps = self.steps.borrow().clone();
        let approvals = self.approvals.borrow().clone();
        let created = run["created_at"].as_i64().unwrap_or(0);
        self.trace_title.set_text(&format!("{} {}", tr("RUN OF"), when_text(created).to_uppercase()));
        let (st, class) = run_status_text(run["status"].as_str().unwrap_or(""));
        let trig = p.triggers.borrow().first().map(|t| m::trigger_summary(&t.config)).unwrap_or_else(|| tr("by hand").to_string());
        let color = match class {
            "ok" => "#8ED2C3",
            "aguarda" => "#E5C98C",
            "falhou" => "#EFA2A6",
            _ => "#8E8F99",
        };
        self.trace_sub.set_markup(&format!("{}: {}  ·  <span foreground=\"{}\">{}</span>", glib::markup_escape_text(tr("trigger")), glib::markup_escape_text(&trig), crate::tema::cor(color), glib::markup_escape_text(&st)));
        while let Some(x) = self.trace.first_child() {
            self.trace.remove(&x);
        }
        // the trigger's row
        if let Some(first) = steps.first() {
            let t = p.triggers.borrow().first().cloned();
            let (name, glyph) = t.as_ref().map(|t| (m::trigger_text(&t.config).0, m::trigger_glyph(t.config["kind"].as_str().unwrap_or("")))).unwrap_or_else(|| (tr("Started by hand").to_string(), "raio"));
            let input: String = m::input_summary(&first.input, &p.doc.borrow().inputs()).chars().take(90).collect();
            self.trace.append(&trace_row(glyph, m::Tone::Gold, &name, &input, &hms(created), "", None));
        }
        let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let totals: std::collections::HashMap<String, usize> = steps.iter().fold(Default::default(), |mut acc, s| {
            *acc.entry(s.node.clone()).or_default() += 1;
            acc
        });
        let chosen_step = self.chosen_step.get();
        let default_step = steps.iter().find(|s| s.kind == "agent").or(steps.last()).map(|s| s.seq).unwrap_or(-1);
        let sel = if steps.iter().any(|s| s.seq == chosen_step) { chosen_step } else { default_step };
        for s in &steps {
            let look = m::look(&s.kind);
            let i = {
                let e = seen.entry(s.node.clone()).or_default();
                *e += 1;
                *e
            };
            let total = totals.get(&s.node).copied().unwrap_or(1);
            let mut title = p.node_title(&s.node);
            if total > 1 {
                title = format!("{title} · {}", trf("{n} of {total}", &[("n", i.to_string()), ("total", total.to_string())]));
            }
            if s.kind == "decide" {
                title = format!("{}: {}", tr("Decide"), title);
            }
            let dur = s.finished.map(|f| f - s.started).filter(|d| *d >= 1).map(|d| m::duration_text(d as f64)).unwrap_or_default();
            let line = step_line(&flow, s);
            let decision = if s.kind == "decide" {
                let n = flow.node(&s.node);
                let route = n.and_then(|n| n.data["route"].as_str()).unwrap_or("");
                let th = n.and_then(|n| n.data["threshold"].as_f64()).unwrap_or(0.8);
                m::parse_decision(s, route).map(|d| (d, th, n.map(|n| n.kind.clone()).unwrap_or_default()))
            } else {
                None
            };
            let time = match (&decision, dur.is_empty()) {
                (Some((d, _, _)), _) if d.millis.is_some() => format!("{} · {} ms", hms(s.started), d.millis.unwrap_or(0)),
                (_, false) => format!("{} · {dur}", hms(s.started)),
                _ => hms(s.started),
            };
            let row = trace_row(look.glyph, look.tone, &title, &line, &time, if s.status == "failed" { "falhou" } else { "" }, decision.as_ref().map(|(d, th, _)| decision_box(d, *th)));
            if s.seq == sel {
                row.add_css_class("escolhido");
            }
            let click = gtk::GestureClick::new();
            let (w, seq) = (Rc::downgrade(self), s.seq);
            click.connect_released(move |_, _, _, _| {
                if let Some(rv) = w.upgrade() {
                    rv.chosen_step.set(seq);
                    rv.render_trace();
                }
            });
            row.add_controller(click);
            self.trace.append(&row);
        }
        for a in &approvals {
            let left = a["expires_at"].as_i64().unwrap_or(0) - now();
            let line = format!("{} · {} {}", tr("waiting for your decision on the approval screen"), tr("expires in"), left_text(left));
            let r = trace_row("escudo", m::Tone::Coral, a["question"].as_str().unwrap_or(""), &line, &hms(now()), "aguarda", None);
            self.trace.append(&r);
        }
        // the chosen step's output, input and instructions
        while let Some(x) = self.detail_tabs.first_child() {
            self.detail_tabs.remove(&x);
        }
        while let Some(x) = self.detail.first_child() {
            self.detail.remove(&x);
        }
        if let Some(s) = steps.iter().find(|s| s.seq == sel).cloned() {
            self.detail_title.set_text(&format!("{}: {}", tr("STEP"), p.node_title(&s.node).to_uppercase()));
            let instructions = flow.node(&s.node).map(|n| match n.kind.as_str() {
                "agent" | "for_each" => n.data["message"].as_str().unwrap_or("").to_string(),
                "decide" => serde_json::to_string_pretty(&n.data["questions"]).unwrap_or_default(),
                _ => String::new(),
            }).unwrap_or_default();
            let mut tabs = vec![("saida", tr("Output").to_string()), ("entrada", tr("Input").to_string())];
            if !instructions.trim().is_empty() {
                tabs.push(("instrucoes", tr("Instructions used").to_string()));
            }
            let cur = self.detail_tab.borrow().clone();
            let cur = if tabs.iter().any(|t| t.0 == cur) { cur } else { "saida".into() };
            let w = Rc::downgrade(self);
            let opts: Vec<(&str, String)> = tabs.iter().map(|(a, b)| (*a, b.clone())).collect();
            self.detail_tabs.append(&crate::fluxo_inspetor::segmented(&opts, &cur, move |t| {
                if let Some(rv) = w.upgrade() {
                    *rv.detail_tab.borrow_mut() = t.to_string();
                    let rv2 = rv.clone();
                    glib::idle_add_local_once(move || rv2.render_trace());
                }
            }));
            let text = match cur.as_str() {
                "entrada" => s.input.clone(),
                "instrucoes" => instructions,
                _ => if s.output.trim().is_empty() { s.detail.clone() } else { s.output.clone() },
            };
            let pretty = serde_json::from_str::<Value>(&text).ok().filter(|v| v.is_object() || v.is_array()).and_then(|v| serde_json::to_string_pretty(&v).ok()).unwrap_or(text);
            let l = rotulo(if pretty.trim().is_empty() { tr("(empty)") } else { &pretty }, "fluxo-mono");
            l.set_xalign(0.0);
            l.set_wrap(true);
            l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            l.set_selectable(true);
            self.detail.append(&l);
        } else {
            self.detail_title.set_text("");
        }
        // the footer
        while let Some(x) = self.footer.first_child() {
            self.footer.remove(&x);
        }
        let btn = |glyph: &str, text: &str, primary: bool| {
            let b = gtk::Button::new();
            b.add_css_class("fluxo-botao");
            if primary {
                b.add_css_class("primario");
            }
            let c = caixa(Orientation::Horizontal, 8, "");
            c.set_halign(Align::Center);
            c.append(&ui_icon(glyph, if primary { "#1C1D24" } else { "#E2E2E6" }, 14));
            c.append(&rotulo(text, ""));
            b.set_child(Some(&c));
            b
        };
        let tl = btn("relogio", tr("See on the timeline"), false);
        let w = Rc::downgrade(&p);
        tl.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.go_to("linha");
            }
        });
        self.footer.append(&tl);
        let again = btn("tocar", tr("Repeat"), false);
        let (w, input) = (Rc::downgrade(&p), steps.first().map(|s| s.input.clone()).unwrap_or_default());
        again.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.run(&input);
            }
        });
        self.footer.append(&again);
        if !approvals.is_empty() {
            let d = btn("escudo", tr("Decide now"), true);
            let (w, run_id) = (Rc::downgrade(&p), run["id"].as_str().unwrap_or("").to_string());
            d.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    follow(&p, &run_id);
                }
            });
            self.footer.append(&d);
        } else if flow.nodes.iter().any(|n| n.kind == "decide") {
            let d = btn("compartilhar", tr("See in the editor"), true);
            let w = Rc::downgrade(&p);
            d.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    p.show_mode("editar");
                }
            });
            self.footer.append(&d);
        }
    }
}

/// Whether a run ended because the person said no (the service records it as done: the flow took the "não" way).
pub fn refused(run: &Value, steps: Option<&[Step]>) -> bool {
    run["status"] == "denied" || (run["status"] == "done" && steps.is_some_and(|s| s.last().is_some_and(|x| x.kind == "approval" && x.detail == "deny")))
}

/// A row of the trace: the block's tile, its name and what it did, its time; a decision's options below.
fn trace_row(glyph: &str, tone: m::Tone, title: &str, line: &str, time: &str, class: &str, extra: Option<gtk::Box>) -> gtk::Box {
    let row = caixa(Orientation::Horizontal, 10, "fluxo-rastro-linha");
    if !class.is_empty() {
        row.add_css_class(class);
    }
    let tile = caixa(Orientation::Horizontal, 0, "fluxo-no-tile");
    tile.add_css_class(tone.class());
    tile.set_size_request(24, 24);
    tile.set_valign(Align::Start);
    let g = if glyph == "spark" { crate::superficies::estrela(12, tone.color()).upcast::<gtk::Widget>() } else { ui_icon(glyph, tone.color(), 14).upcast() };
    g.set_hexpand(true);
    tile.append(&g);
    tile.set_hexpand(false);
    row.append(&tile);
    let col = caixa(Orientation::Vertical, 2, "");
    col.set_hexpand(true);
    let top = caixa(Orientation::Horizontal, 8, "");
    let t = rotulo(title, "fluxo-rastro-titulo");
    t.set_xalign(0.0);
    t.set_hexpand(true);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    top.append(&t);
    let tm = rotulo(time, "fluxo-tabela-mono");
    tm.set_valign(Align::Start);
    top.append(&tm);
    col.append(&top);
    if !line.is_empty() {
        let l = rotulo(line, "fluxo-rastro-linha-texto");
        if !class.is_empty() {
            l.add_css_class(class);
        }
        l.set_xalign(0.0);
        l.set_wrap(true);
        l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        l.set_lines(3);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        col.append(&l);
    }
    if let Some(e) = extra {
        col.append(&e);
    }
    row.append(&col);
    row
}

/// A decision's options with their probabilities, the confidence against the threshold and the way it went.
fn decision_box(d: &m::Decision, threshold: f64) -> gtk::Box {
    let b = caixa(Orientation::Vertical, 6, "fluxo-decisao-caixa");
    for (k, p) in &d.options {
        let r = caixa(Orientation::Horizontal, 10, "");
        let name = m::way_label("decide", Some(k));
        let l = rotulo(if name.is_empty() { k } else { &name }, "fluxo-opcao-texto");
        l.set_xalign(0.0);
        l.set_hexpand(true);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        r.append(&l);
        let bar = gtk::LevelBar::for_interval(0.0, 1.0);
        bar.add_css_class("fluxo-barra");
        if *k == d.chosen {
            bar.add_css_class("escolhida");
        }
        bar.set_value(*p);
        bar.set_size_request(140, -1);
        bar.set_valign(Align::Center);
        r.append(&bar);
        let pct = rotulo(&format!("{}%", (p * 100.0).round()), "fluxo-pct");
        pct.set_width_chars(4);
        pct.set_xalign(1.0);
        r.append(&pct);
        b.append(&r);
    }
    let foot = caixa(Orientation::Horizontal, 8, "");
    let ok = d.confidence >= threshold;
    let chip = rotulo(&format!("{} {} {} {}", tr("confidence"), m::decimal(d.confidence, 2), if ok { "≥" } else { "<" }, m::decimal(threshold, 2)), "fluxo-resultado");
    chip.add_css_class(if ok { "ok" } else { "aguarda" });
    foot.append(&chip);
    let way = m::way_label("decide", Some(&d.chosen));
    let went = if d.chosen == m::LOW_CONFIDENCE { tr("→ asked you").to_string() } else { format!("→ {} \"{}\"", tr("went by"), if way.is_empty() { d.chosen.clone() } else { way }) };
    let wl = rotulo(&went, "fluxo-chave-sub");
    wl.set_valign(Align::Center);
    foot.append(&wl);
    if d.cached {
        foot.append(&rotulo(tr("(cache)"), "fluxo-chave-sub"));
    }
    b.append(&foot);
    b
}

/// A narrow window (design "Janelas estreitas"): no palette (a "+" over the canvas opens it), the inspector a
/// sheet under the canvas, the top bar's buttons fold, the runs' trace under the list.
pub fn apply_narrow(page: &Rc<FlowPage>, narrow: bool) {
    let edit = page.canvas.root.parent().and_downcast::<gtk::Box>();
    if let Some(edit) = edit {
        edit.set_orientation(if narrow { Orientation::Vertical } else { Orientation::Horizontal });
    }
    page.inspector.set_size_request(if narrow { -1 } else { 338 }, if narrow { 330 } else { -1 });
    // (the sheet keeps its height: the canvas takes the rest)
    page.inspector.set_vexpand(!narrow);
    if narrow { page.inspector.add_css_class("folha") } else { page.inspector.remove_css_class("folha") }
    page.update_top();
    // the palette goes into the "+" over the canvas, and back
    let pal = page.palette_widget().clone();
    if let Some(parent) = pal.parent() {
        if let Some(b) = parent.downcast_ref::<gtk::Box>() {
            b.remove(&pal);
        } else if let Some(pop) = parent.downcast_ref::<gtk::Popover>() {
            pop.set_child(None::<&gtk::Widget>);
        }
    }
    pal.set_visible(true);
    if narrow {
        pal.set_size_request(240, 420);
        page.canvas.add_button.popover().and_downcast::<gtk::Popover>().unwrap().set_child(Some(&pal));
    } else {
        pal.set_size_request(196, -1);
        if let Some(edit) = page.canvas.root.parent().and_downcast::<gtk::Box>() {
            edit.prepend(&pal);
        }
    }
    page.canvas.set_narrow(narrow);
    if let Some(rv) = page.runs_view.borrow().as_ref() {
        rv.root.set_orientation(if narrow { Orientation::Vertical } else { Orientation::Horizontal });
        rv.side.set_size_request(if narrow { -1 } else { 500 }, if narrow { 380 } else { -1 });
        rv.side.set_vexpand(!narrow);
    }
    page.refresh_inspector();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_left_reads_short() {
        assert_eq!(left_text(7000), "1 h 56 min");
        assert_eq!(left_text(240), "4 min");
        assert_eq!(left_text(-5), "0 s");
    }

    #[test]
    fn live_runs_are_active_until_they_end() {
        let mut l = Live { status: "pending".into(), ..Default::default() };
        assert!(l.active());
        l.status = "waiting_approval".into();
        assert!(l.active());
        l.status = "done".into();
        assert!(!l.active());
    }

    #[test]
    fn a_no_ends_a_run_as_refused() {
        let deny = Step { kind: "approval".into(), detail: "deny".into(), ..Default::default() };
        let ok = Step { kind: "notify".into(), ..Default::default() };
        assert!(refused(&json!({"status": "done"}), Some(std::slice::from_ref(&deny))));
        assert!(!refused(&json!({"status": "done"}), Some(&[deny, ok])));
        assert!(refused(&json!({"status": "denied"}), None));
        assert!(!refused(&json!({"status": "done"}), None));
    }

    #[test]
    fn trace_lines() {
        let f = FlowDoc::from_json(&json!({"nodes": [
            {"id": "c", "type": "condition", "data": {"conditions": [{"id": "sim", "data": {"field": "var:tipo", "operator": "equals", "value": "fatura"}}]}},
            {"id": "a", "type": "approval", "data": {"question": "Q"}}
        ], "edges": []}));
        let s = Step { node: "c".into(), kind: "condition".into(), status: "done".into(), detail: "→ sim".into(), ..Default::default() };
        assert_eq!(step_line(&f, &s), "{tipo} == \"fatura\" → sim");
        let a = Step { node: "a".into(), kind: "approval".into(), status: "done".into(), detail: "deny".into(), ..Default::default() };
        assert_eq!(step_line(&f, &a), tr("refused by you"));
    }
}
