//! The agent's browser (design "Chat · navegador do agente"): in the conversation's side bar, the pages the agent
//! reads, as a browser draws them — the part it read outlined ("lendo"), each step a thumbnail to go back in time, and
//! "Assumir o controle": the person's clicks, scrolling and typing go to the same page, without reloading it.
//!
//! The pages are drawn by kalangod's Moli (`kalango/navegador`): held like the agents' browser — its own profile (none
//! of the person's logins), local addresses refused, a jail and limits —, never by this process. Only the pictures
//! come here.
use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

/// The page's size in the browser (the side bar shows it scaled to its width).
const PAGE_W: f64 = 1100.0;
const PAGE_H: f64 = 760.0;

enum StepKind {
    Search { query: String, results: Vec<(String, String)> },
    /// a page read, with its text (lowercase: to find the one an answer came from)
    Read { url: String, title: String, text: String },
}

struct Step {
    kind: StepKind,
    picture: Option<gdk::Texture>,
}

impl Step {
    fn label(&self, n: usize) -> String {
        match &self.kind {
            StepKind::Search { .. } => trf("{n} · searched", &[("n", n.to_string())]),
            StepKind::Read { url, .. } => trf("{n} · read {site}", &[("n", n.to_string()), ("site", host(url))]),
        }
    }
}

/// What the browser is asked to do (one at a time: kalangod's browser is one page).
enum Job {
    Open(String, Option<usize>),
    Outline(Vec<String>),
    Click(f64, f64),
    Scroll(f64),
    Type(String),
    Key(&'static str),
    Back,
    Forward,
    Reload,
}

#[derive(Default)]
struct Live {
    url: String,
    title: String,
    picture: Option<gdk::Texture>,
    back: bool,
    forward: bool,
}

#[derive(Default)]
struct State {
    conversation: String,
    processed: usize,
    steps: Vec<Step>,
    /// a step's picture shown instead of the live page (going back in time)
    shown: Option<usize>,
    live: Live,
    /// the part the agent read, in the page's coordinates, and its words
    outline: Option<(f64, f64, f64, f64)>,
    control: bool,
    working: bool,
    running: bool,
    jobs: VecDeque<Job>,
    /// the step being read now (for "passo N de M")
    reading: Option<String>,
    closed: Vec<String>,
}

pub struct AgentBrowser {
    pub raiz: gtk::Box,
    tabs: gtk::Box,
    back: gtk::Button,
    forward: gtk::Button,
    address: gtk::Entry,
    view: gtk::Overlay,
    picture: gtk::Picture,
    marks: gtk::DrawingArea,
    empty: gtk::Label,
    band_title: gtk::Label,
    band_sub: gtk::Label,
    pause: gtk::Button,
    take: gtk::Button,
    take_label: gtk::Label,
    strip: gtk::Box,
    state: RefCell<State>,
    typed: RefCell<String>,
    scrolled: Cell<f64>,
    /// stops the agent (the conversation's)
    pub on_pause: RefCell<Option<Box<dyn Fn()>>>,
}

fn host(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or(url).trim_start_matches("www.").to_string()
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// The words of an answer that point at what was read: names, numbers, terms with capitals.
fn answer_words(answer: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in answer.split(|c: char| !c.is_alphanumeric() && c != '\'' && c != '-') {
        let w = w.trim_matches(|c: char| c == '\'' || c == '-');
        // (a name of three letters or more; a number of two digits or more: "28'", "2026")
        let n = w.chars().count();
        let strong = (n >= 3 && w.chars().next().is_some_and(char::is_uppercase)) || (n >= 2 && w.chars().any(|c| c.is_ascii_digit()));
        if strong && !out.iter().any(|x| x.eq_ignore_ascii_case(w)) {
            out.push(w.to_string());
        }
        if out.len() >= 14 {
            break;
        }
    }
    out
}

fn texture(b64: &str) -> Option<gdk::Texture> {
    let bytes = glib::base64_decode(b64);
    gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)).ok()
}

impl AgentBrowser {
    pub fn new() -> Rc<AgentBrowser> {
        let raiz = caixa(Orientation::Vertical, 10, "navegador");
        raiz.set_margin_top(12);
        raiz.set_margin_end(8);
        // the pages opened, and that the session is not the person's
        let top = caixa(Orientation::Horizontal, 6, "");
        let tabs = caixa(Orientation::Horizontal, 6, "");
        let tabs_scroll = gtk::ScrolledWindow::new();
        tabs_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        tabs_scroll.set_hexpand(true);
        tabs_scroll.set_child(Some(&tabs));
        top.append(&tabs_scroll);
        // "+": a page of the person's own (they take the control and type its address)
        let new_tab = gtk::Button::new();
        new_tab.add_css_class("central-controle");
        new_tab.set_child(Some(&ui_icon("mais", "#8E8F99", 13)));
        new_tab.set_tooltip_text(Some(tr("Open a page")));
        new_tab.set_valign(Align::Center);
        top.append(&new_tab);
        crate::central::posicao_debug_sempre("navegador nova pagina", new_tab.upcast_ref());
        let session = rotulo(tr("isolated session · none of your logins"), "navegador-sessao");
        session.set_valign(Align::Center);
        session.set_ellipsize(gtk::pango::EllipsizeMode::End);
        session.set_max_width_chars(34);
        session.set_width_chars(6);
        top.append(&session);
        raiz.append(&top);
        // the address bar
        let bar = caixa(Orientation::Horizontal, 4, "navegador-endereco");
        let nav = |icon: &str, tip: &str| {
            let b = gtk::Button::new();
            b.add_css_class("central-controle");
            b.set_child(Some(&ui_icon(icon, "#8E8F99", 14)));
            b.set_tooltip_text(Some(tip));
            b
        };
        let back = nav("seta-esquerda", tr("Back"));
        let forward = nav("seta-direita", tr("Forward"));
        let reload = nav("reiniciar", tr("Reload"));
        bar.append(&back);
        bar.append(&forward);
        bar.append(&reload);
        let lock = ui_icon("cadeado", "#8E8F99", 12);
        lock.set_margin_start(6);
        bar.append(&lock);
        // (the address: read-only while the agent drives; the person in control types one, Enter opens it)
        let address = gtk::Entry::new();
        address.add_css_class("navegador-url");
        address.set_hexpand(true);
        address.set_editable(false);
        address.set_has_frame(false);
        bar.append(&address);
        raiz.append(&bar);
        // the page
        let view = gtk::Overlay::new();
        view.add_css_class("navegador-vista");
        view.set_vexpand(true);
        view.set_focusable(true);
        view.set_overflow(gtk::Overflow::Hidden);
        let picture = gtk::Picture::new();
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_valign(Align::Fill);
        view.set_child(Some(&picture));
        let marks = gtk::DrawingArea::new();
        marks.set_can_target(false);
        view.add_overlay(&marks);
        let empty = rotulo(tr("The pages the agent reads in this conversation show here, as a browser draws them."), "central-agente-sub");
        empty.set_wrap(true);
        empty.set_justify(gtk::Justification::Center);
        empty.set_valign(Align::Center);
        empty.set_margin_start(24);
        empty.set_margin_end(24);
        view.add_overlay(&empty);
        raiz.append(&view);
        // what the agent is doing, and the person's way in
        let band = caixa(Orientation::Horizontal, 10, "navegador-faixa");
        band.append(&ui_icon("spark", "#A5A1E2", 14));
        let texts = caixa(Orientation::Horizontal, 8, "");
        texts.set_hexpand(true);
        let band_title = rotulo("", "navegador-faixa-titulo");
        band_title.set_xalign(0.0);
        band_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        band_title.set_max_width_chars(24);
        let band_sub = rotulo("", "central-agente-sub");
        band_sub.set_xalign(0.0);
        band_sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
        band_sub.set_hexpand(true);
        band_sub.set_max_width_chars(1);
        texts.append(&band_title);
        texts.append(&band_sub);
        band.append(&texts);
        let pause = gtk::Button::new();
        pause.add_css_class("central-controle");
        let pb = caixa(Orientation::Horizontal, 6, "");
        pb.append(&ui_icon("pausa", "#C9CAD1", 12));
        pb.append(&rotulo(tr("Pause"), ""));
        pause.set_child(Some(&pb));
        band.append(&pause);
        let take = gtk::Button::new();
        take.add_css_class("navegador-assumir");
        let tb = caixa(Orientation::Horizontal, 6, "");
        tb.append(&ui_icon("ponteiro", "#1C1D24", 12));
        let take_label = rotulo(tr("Take control"), "");
        tb.append(&take_label);
        take.set_child(Some(&tb));
        band.append(&take);
        raiz.append(&band);
        // the steps, to go back in time
        let strip = caixa(Orientation::Horizontal, 8, "");
        let strip_scroll = gtk::ScrolledWindow::new();
        strip_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        strip_scroll.set_child(Some(&strip));
        strip_scroll.set_min_content_height(84);
        raiz.append(&strip_scroll);

        let b = Rc::new(AgentBrowser {
            raiz, tabs, back: back.clone(), forward: forward.clone(), address, view: view.clone(), picture, marks: marks.clone(), empty, band_title, band_sub,
            pause: pause.clone(), take: take.clone(), take_label, strip, state: RefCell::default(), typed: RefCell::default(), scrolled: Cell::new(0.0),
            on_pause: RefCell::default(),
        });
        let w = Rc::downgrade(&b);
        marks.set_draw_func(move |_, cr, aw, ah| {
            if let Some(b) = w.upgrade() {
                b.draw_marks(cr, f64::from(aw), f64::from(ah));
            }
        });
        for (btn, job) in [(&back, 0), (&forward, 1), (&reload, 2)] {
            let w = Rc::downgrade(&b);
            btn.connect_clicked(move |_| {
                if let Some(b) = w.upgrade() {
                    if debug() {
                        eprintln!("debug navegador: botão {}", ["voltar", "avançar", "recarregar"][job]);
                    }
                    b.push(match job {
                        0 => Job::Back,
                        1 => Job::Forward,
                        _ => Job::Reload,
                    });
                }
            });
        }
        let w = Rc::downgrade(&b);
        new_tab.connect_clicked(move |_| {
            if let Some(b) = w.upgrade() {
                if !b.state.borrow().control {
                    b.set_control(true);
                }
                b.address.set_editable(true);
                // (after the control's own focus moves: the address is where the person types now)
                let w = Rc::downgrade(&b);
                glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
                    if let Some(b) = w.upgrade() {
                        b.address.grab_focus();
                        b.address.select_region(0, -1);
                        if debug() {
                            eprintln!("debug navegador: endereço para digitar (editável {}, controle {})", b.address.is_editable(), b.state.borrow().control);
                        }
                    }
                });
            }
        });
        let w = Rc::downgrade(&b);
        b.address.connect_activate(move |e| {
            let Some(b) = w.upgrade() else { return };
            let t = e.text().trim().to_string();
            if debug() {
                eprintln!("debug navegador: endereço digitado «{t}»");
            }
            if t.is_empty() || !b.state.borrow().control {
                return;
            }
            let url = if t.contains("://") { t } else { format!("https://{t}") };
            b.push(Job::Open(url, None));
            b.view.grab_focus();
        });
        let w = Rc::downgrade(&b);
        pause.connect_clicked(move |_| {
            if let Some(b) = w.upgrade() {
                if let Some(f) = b.on_pause.borrow().as_ref() {
                    f();
                }
            }
        });
        let w = Rc::downgrade(&b);
        take.connect_clicked(move |_| {
            if let Some(b) = w.upgrade() {
                let on = !b.state.borrow().control;
                b.set_control(on);
            }
        });
        // the person's hand on the page (in control)
        let click = gtk::GestureClick::new();
        let w = Rc::downgrade(&b);
        click.connect_released(move |_, _, x, y| {
            if let Some(b) = w.upgrade() {
                b.view.grab_focus();
                if let Some((px, py)) = b.to_page(x, y).filter(|_| b.state.borrow().control) {
                    b.push(Job::Click(px, py));
                }
            }
        });
        view.add_controller(click);
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let w = Rc::downgrade(&b);
        scroll.connect_scroll(move |_, _, dy| {
            let Some(b) = w.upgrade() else { return glib::Propagation::Proceed };
            if !b.state.borrow().control {
                return glib::Propagation::Proceed;
            }
            // (a wheel's turns, gathered for a moment: one request)
            let first = b.scrolled.get() == 0.0;
            b.scrolled.set(b.scrolled.get() + dy * 120.0);
            if first {
                let w = Rc::downgrade(&b);
                glib::timeout_add_local_once(std::time::Duration::from_millis(160), move || {
                    if let Some(b) = w.upgrade() {
                        let d = b.scrolled.replace(0.0);
                        b.push(Job::Scroll(d));
                    }
                });
            }
            glib::Propagation::Stop
        });
        view.add_controller(scroll);
        let keys = gtk::EventControllerKey::new();
        let w = Rc::downgrade(&b);
        keys.connect_key_pressed(move |_, k, _, _| {
            let Some(b) = w.upgrade() else { return glib::Propagation::Proceed };
            if !b.state.borrow().control {
                return glib::Propagation::Proceed;
            }
            let special = match k {
                gdk::Key::Return | gdk::Key::KP_Enter => Some("Enter"),
                gdk::Key::Tab => Some("Tab"),
                gdk::Key::BackSpace => Some("Backspace"),
                gdk::Key::Escape => Some("Escape"),
                gdk::Key::Down => Some("ArrowDown"),
                gdk::Key::Up => Some("ArrowUp"),
                gdk::Key::Page_Down => Some("PageDown"),
                gdk::Key::Page_Up => Some("PageUp"),
                _ => None,
            };
            if let Some(name) = special {
                b.flush_typed();
                b.push(Job::Key(name));
                return glib::Propagation::Stop;
            }
            if let Some(c) = k.to_unicode().filter(|c| !c.is_control()) {
                // (letters typed in a row: one request)
                let first = b.typed.borrow().is_empty();
                b.typed.borrow_mut().push(c);
                if first {
                    let w = Rc::downgrade(&b);
                    glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                        if let Some(b) = w.upgrade() {
                            b.flush_typed();
                        }
                    });
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        view.add_controller(keys);
        crate::central::posicao_debug_sempre("navegador assumir", take.upcast_ref());
        crate::central::posicao_debug_sempre("navegador pausar", pause.upcast_ref());
        crate::central::posicao_debug_sempre("navegador voltar", back.upcast_ref());
        crate::central::posicao_debug_sempre("navegador pagina", view.upcast_ref());
        b.render();
        b
    }

    /// A page opened from the conversation (a source, or one that asks for a login: then the person drives).
    pub fn show(self: &Rc<Self>, url: &str, control: bool) {
        self.state.borrow_mut().shown = None;
        self.push(Job::Open(url.to_string(), None));
        if control && !self.state.borrow().control {
            self.set_control(true);
        }
    }

    fn flush_typed(self: &Rc<Self>) {
        let t = self.typed.take();
        if !t.is_empty() {
            self.push(Job::Type(t));
        }
    }

    /// A point of the view, in the page's coordinates (the picture is scaled to fit, centred).
    fn to_page(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let (aw, ah) = (f64::from(self.view.width()), f64::from(self.view.height()));
        let s = (aw / PAGE_W).min(ah / PAGE_H);
        if s <= 0.0 {
            return None;
        }
        let (ox, oy) = ((aw - PAGE_W * s) / 2.0, (ah - PAGE_H * s) / 2.0);
        let (px, py) = ((x - ox) / s, (y - oy) / s);
        ((0.0..=PAGE_W).contains(&px) && (0.0..=PAGE_H).contains(&py)).then_some((px, py))
    }

    fn draw_marks(&self, cr: &gtk::cairo::Context, aw: f64, ah: f64) {
        let st = self.state.borrow();
        let s = (aw / PAGE_W).min(ah / PAGE_H);
        let (ox, oy) = ((aw - PAGE_W * s) / 2.0, (ah - PAGE_H * s) / 2.0);
        // the part read: a lavender outline and its tag
        if let (Some((x, y, w, h)), None) = (st.outline, st.shown) {
            let (rx, ry, rw, rh) = (ox + x * s - 4.0, oy + y * s - 4.0, w * s + 8.0, h * s + 8.0);
            cr.set_source_rgba(165.0 / 255.0, 161.0 / 255.0, 226.0 / 255.0, 0.95);
            cr.set_line_width(1.5);
            cr.rectangle(rx, ry, rw, rh);
            let _ = cr.stroke();
            let tag = tr("reading");
            cr.select_font_face("Inter", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
            cr.set_font_size(10.0);
            let tw = cr.text_extents(tag).map(|e| e.width()).unwrap_or(40.0);
            let ty = (ry - 18.0).max(oy);
            cr.rectangle(rx, ty, tw + 14.0, 16.0);
            let _ = cr.fill();
            cr.set_source_rgb(28.0 / 255.0, 29.0 / 255.0, 36.0 / 255.0);
            cr.move_to(rx + 7.0, ty + 11.5);
            let _ = cr.show_text(tag);
        }
        // in control: a thin frame says the page is the person's now
        if st.control {
            cr.set_source_rgba(142.0 / 255.0, 210.0 / 255.0, 195.0 / 255.0, 0.9);
            cr.set_line_width(2.0);
            cr.rectangle(ox + 1.0, oy + 1.0, PAGE_W * s - 2.0, PAGE_H * s - 2.0);
            let _ = cr.stroke();
        }
    }

    fn set_control(self: &Rc<Self>, on: bool) {
        self.state.borrow_mut().control = on;
        if on {
            // the agent waits while the person drives
            if self.state.borrow().running {
                if let Some(f) = self.on_pause.borrow().as_ref() {
                    f();
                }
            }
            self.view.grab_focus();
        }
        if debug() {
            eprintln!("debug navegador: {}", if on { "no controle da pessoa" } else { "controle devolvido" });
        }
        self.render();
    }

    /// The conversation's events: the pages the agent searched and read (new ones open here) and, after its answer,
    /// the part of the page it used.
    pub fn feed(self: &Rc<Self>, conversation: &str, events: &[Value], running: bool) {
        let mut opens: Vec<(String, usize)> = Vec::new();
        let mut outline: Option<(Option<String>, Vec<String>)> = None;
        {
            let mut st = self.state.borrow_mut();
            if st.conversation != conversation {
                *st = State { conversation: conversation.to_string(), ..Default::default() };
            }
            st.running = running;
            let replay = st.processed == 0;
            let mut query = String::new();
            let start = st.processed.min(events.len());
            for e in &events[start..] {
                let name = e["nome"].as_str().unwrap_or("");
                match e["tipo"].as_str() {
                    Some("ferramenta") if name == "web.buscar" => query = e["argumentos"]["consulta"].as_str().unwrap_or("").to_string(),
                    Some("ferramenta") if name == "web.ler" => st.reading = e["argumentos"]["url"].as_str().map(host),
                    Some("resultado") if e["ok"] == true && name.starts_with("web.") => {
                        let Ok(v) = serde_json::from_str::<Value>(e["texto"].as_str().unwrap_or("")) else { continue };
                        if let Some(rs) = v["resultados"].as_array() {
                            let results = rs.iter().filter_map(|r| Some((r["titulo"].as_str()?.to_string(), r["url"].as_str()?.to_string()))).collect();
                            let q = v["consulta"].as_str().map(str::to_string).unwrap_or_else(|| query.clone());
                            st.steps.push(Step { kind: StepKind::Search { query: q, results }, picture: None });
                        } else if let Some(url) = v["url"].as_str() {
                            let title = v["titulo"].as_str().unwrap_or("").to_string();
                            let text = v["texto"].as_str().unwrap_or("").to_lowercase();
                            st.steps.push(Step { kind: StepKind::Read { url: url.to_string(), title, text }, picture: None });
                            let i = st.steps.len() - 1;
                            opens.push((url.to_string(), i));
                        }
                        st.reading = None;
                    }
                    Some("resposta") if st.steps.iter().any(|s| matches!(s.kind, StepKind::Read { .. })) => {
                        let words = answer_words(e["texto"].as_str().unwrap_or(""));
                        // the page the answer came from: the one with most of its words (not always the last read)
                        let best = st.steps.iter().filter_map(|s| match &s.kind {
                            StepKind::Read { url, text, .. } => Some((words.iter().filter(|w| text.contains(&w.to_lowercase())).count(), url.clone())),
                            _ => None,
                        }).max_by_key(|(n, _)| *n).map(|(_, u)| u);
                        outline = Some((best, words));
                    }
                    _ => {}
                }
            }
            st.processed = events.len();
            // (a conversation opened again: only its last page, not every page it ever read)
            if replay && opens.len() > 1 {
                opens = opens.split_off(opens.len() - 1);
            }
        }
        for (url, i) in opens.clone() {
            self.push(Job::Open(url, Some(i)));
        }
        if let Some((best, words)) = outline.filter(|(_, w)| !w.is_empty()) {
            let last = opens.last().map(|(u, _)| u.clone()).unwrap_or_else(|| self.state.borrow().live.url.clone());
            if let Some(u) = best.filter(|u| *u != last) {
                self.push(Job::Open(u, None));
            }
            self.push(Job::Outline(words));
        }
        self.render();
    }

    fn push(self: &Rc<Self>, job: Job) {
        self.state.borrow_mut().jobs.push_back(job);
        self.next();
    }

    /// The next request to kalangod's browser, when none is going.
    fn next(self: &Rc<Self>) {
        let job = {
            let mut st = self.state.borrow_mut();
            if st.working {
                return;
            }
            let Some(j) = st.jobs.pop_front() else { return };
            st.working = true;
            j
        };
        let (params, step) = match &job {
            Job::Open(url, step) => (json!({"acao": "abrir", "url": url, "largura": PAGE_W, "altura": PAGE_H}), *step),
            Job::Outline(words) => (json!({"acao": "destacar", "palavras": words}), None),
            Job::Click(x, y) => (json!({"acao": "clicar", "x": x, "y": y}), None),
            Job::Scroll(dy) => (json!({"acao": "rolar", "dy": dy}), None),
            Job::Type(t) => (json!({"acao": "digitar", "texto": t}), None),
            Job::Key(k) => (json!({"acao": "tecla", "tecla": k}), None),
            Job::Back => (json!({"acao": "voltar"}), None),
            Job::Forward => (json!({"acao": "avancar"}), None),
            Job::Reload => (json!({"acao": "recarregar"}), None),
        };
        let outline = matches!(job, Job::Outline(_));
        let acao = params["acao"].as_str().unwrap_or("").to_string();
        self.render();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_navegador(params), move |r: Result<Value, String>| {
            let Some(b) = w.upgrade() else { return };
            b.state.borrow_mut().working = false;
            match r {
                Ok(v) => b.apply(&v, step, outline, &acao),
                Err(e) => {
                    if debug() {
                        eprintln!("debug navegador: {acao} falhou: {e}");
                    }
                    b.band_sub.set_text(&trf("the browser could not: {error}", &[("error", e)]));
                }
            }
            b.next();
            b.render();
        });
    }

    fn apply(&self, v: &Value, step: Option<usize>, outline: bool, acao: &str) {
        let pic = v["foto"].as_str().and_then(texture);
        let mut st = self.state.borrow_mut();
        st.live = Live { url: v["url"].as_str().unwrap_or("").to_string(), title: v["titulo"].as_str().unwrap_or("").to_string(), picture: pic.clone(),
            back: v["voltar"].as_bool().unwrap_or(false), forward: v["avancar"].as_bool().unwrap_or(false) };
        if let Some(i) = step {
            if let Some(s) = st.steps.get_mut(i) {
                s.picture = pic;
            }
            st.shown = None;
        }
        if outline {
            st.outline = v["destaque"]["x"].as_f64().map(|x| (x, v["destaque"]["y"].as_f64().unwrap_or(0.0), v["destaque"]["w"].as_f64().unwrap_or(0.0), v["destaque"]["h"].as_f64().unwrap_or(0.0)));
            if debug() {
                eprintln!("debug navegador: destaque «{}» em {:?}", v["destaque"]["trecho"].as_str().unwrap_or("(nada)").chars().take(80).collect::<String>().replace('\n', " "), st.outline);
            }
        } else if acao != "destacar" {
            // (the page changed: the outline belonged to the one before)
            st.outline = None;
        }
        if debug() {
            let lidos = st.steps.iter().filter(|s| matches!(s.kind, StepKind::Read { .. })).count();
            eprintln!("debug navegador: {acao} → «{}» ({}) · {} passos, {lidos} páginas", st.live.title, st.live.url, st.steps.len());
        }
    }

    /// Everything on screen from the state.
    fn render(self: &Rc<Self>) {
        let st = self.state.borrow();
        // the picture: a step's (back in time) or the live page
        let shown_pic = st.shown.and_then(|i| st.steps.get(i)).and_then(|s| s.picture.clone()).or_else(|| st.live.picture.clone());
        self.picture.set_paintable(shown_pic.as_ref());
        self.empty.set_visible(shown_pic.is_none());
        self.address.set_editable(st.control);
        // (the entry's text has the focus, not the entry itself)
        if !self.address.state_flags().contains(gtk::StateFlags::FOCUS_WITHIN) {
            self.address.set_text(&st.live.url);
        }
        let back_on = st.live.back && !st.working;
        if debug() && self.back.is_sensitive() != back_on {
            eprintln!("debug navegador: voltar ativo {back_on}");
        }
        self.back.set_sensitive(back_on);
        self.forward.set_sensitive(st.live.forward && !st.working);
        self.marks.queue_draw();
        // the pages as tabs
        while let Some(c) = self.tabs.first_child() {
            self.tabs.remove(&c);
        }
        let mut seen: Vec<String> = Vec::new();
        for s in st.steps.iter().rev() {
            if let StepKind::Read { url, title, .. } = &s.kind {
                if seen.contains(url) || st.closed.contains(url) || seen.len() >= 5 {
                    continue;
                }
                seen.push(url.clone());
                let chip = caixa(Orientation::Horizontal, 6, "navegador-aba");
                if *url == st.live.url {
                    chip.add_css_class("ativa");
                }
                chip.append(&ui_icon("globo", "#8E8F99", 11));
                let t = rotulo(if title.is_empty() { url } else { title }, "");
                t.set_ellipsize(gtk::pango::EllipsizeMode::End);
                t.set_max_width_chars(18);
                chip.append(&t);
                let x = gtk::Button::new();
                x.add_css_class("navegador-aba-fechar");
                x.set_child(Some(&rotulo("×", "")));
                chip.append(&x);
                let open = gtk::GestureClick::new();
                let (w, u) = (Rc::downgrade(self), url.clone());
                open.connect_released(move |_, _, _, _| {
                    if let Some(b) = w.upgrade() {
                        b.push(Job::Open(u.clone(), None));
                    }
                });
                t.add_controller(open);
                let (w, u) = (Rc::downgrade(self), url.clone());
                x.connect_clicked(move |_| {
                    if let Some(b) = w.upgrade() {
                        b.state.borrow_mut().closed.push(u.clone());
                        b.render();
                    }
                });
                self.tabs.append(&chip);
            }
        }
        // the band
        let n = st.steps.len();
        if st.control {
            self.band_title.set_text(tr("You are in control"));
            self.band_sub.set_text(tr("your clicks and typing go to the page · the agent waits"));
            self.take_label.set_text(tr("Give back to the agent"));
        } else {
            self.take_label.set_text(tr("Take control"));
            if st.running {
                self.band_title.set_text(tr("The agent is browsing"));
                let doing = match &st.reading {
                    Some(h) => trf("reading {site}", &[("site", h.clone())]),
                    None => st.steps.last().map(|s| s.label(n)).unwrap_or_default(),
                };
                self.band_sub.set_text(&trf("step {i} · {doing}", &[("i", (n + usize::from(st.reading.is_some())).max(1).to_string()), ("doing", doing)]));
            } else if n > 0 {
                let pages = st.steps.iter().filter(|s| matches!(s.kind, StepKind::Read { .. })).count();
                self.band_title.set_text(tr("What the agent read"));
                self.band_sub.set_text(&trf("{n} steps · {p} pages", &[("n", n.to_string()), ("p", pages.to_string())]));
            } else {
                self.band_title.set_text(tr("The agent's browser"));
                self.band_sub.set_text(tr("nothing read in this conversation yet"));
            }
        }
        if st.working && !st.control {
            self.band_sub.set_text(&format!("{} · {}", self.band_sub.text(), tr("loading…")));
        }
        self.pause.set_visible(st.running && !st.control);
        self.take.set_sensitive(st.live.picture.is_some());
        // (their places for the session test: measured again, since the panel widens with a page in it)
        for (nome, w) in [("navegador assumir", self.take.upcast_ref::<gtk::Widget>()), ("navegador voltar", self.back.upcast_ref()), ("navegador pausar", self.pause.upcast_ref()),
            ("navegador pagina", self.view.upcast_ref())] {
            if w.is_visible() {
                crate::central::posicao_debug(nome, w);
            }
        }
        // the steps
        while let Some(c) = self.strip.first_child() {
            self.strip.remove(&c);
        }
        for (i, s) in st.steps.iter().enumerate() {
            let card = caixa(Orientation::Vertical, 4, "");
            let thumb = gtk::Button::new();
            thumb.add_css_class("navegador-passo");
            if st.shown == Some(i) || (st.shown.is_none() && matches!(&s.kind, StepKind::Read { url, .. } if *url == st.live.url) && i + 1 == n) {
                thumb.add_css_class("ativo");
            }
            thumb.set_size_request(118, 64);
            match (&s.picture, &s.kind) {
                (Some(p), _) => {
                    let pic = gtk::Picture::for_paintable(p);
                    pic.set_content_fit(gtk::ContentFit::Cover);
                    pic.set_can_shrink(true);
                    thumb.set_child(Some(&pic));
                }
                (None, StepKind::Search { query, .. }) => {
                    let l = rotulo(&format!("“{query}”"), "navegador-passo-busca");
                    l.set_wrap(true);
                    l.set_lines(3);
                    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    l.set_max_width_chars(14);
                    thumb.set_child(Some(&l));
                }
                (None, StepKind::Read { url, .. }) => {
                    let l = rotulo(&host(url), "navegador-passo-busca");
                    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    l.set_max_width_chars(14);
                    thumb.set_child(Some(&l));
                }
            }
            if let StepKind::Search { results, .. } = &s.kind {
                let lista: Vec<String> = results.iter().take(5).map(|(t, _)| format!("• {t}")).collect();
                thumb.set_tooltip_text(Some(&lista.join("\n")));
            }
            let w = Rc::downgrade(self);
            thumb.connect_clicked(move |_| {
                if let Some(b) = w.upgrade() {
                    b.go_to_step(i);
                }
            });
            card.append(&thumb);
            let l = rotulo(&s.label(i + 1), "central-agente-sub");
            l.set_xalign(0.0);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.set_max_width_chars(16);
            card.append(&l);
            self.strip.append(&card);
            if i == n - 1 {
                crate::central::posicao_debug(&format!("navegador passo {}", i + 1), thumb.upcast_ref());
            }
        }
    }

    /// A step's thumbnail: its picture (back in time); a page never drawn opens; the newest returns to the live page.
    fn go_to_step(self: &Rc<Self>, i: usize) {
        let open = {
            let mut st = self.state.borrow_mut();
            let last = st.steps.len().saturating_sub(1);
            match st.steps.get(i) {
                Some(Step { picture: Some(_), .. }) if i != last => {
                    st.shown = Some(i);
                    None
                }
                Some(Step { kind: StepKind::Read { url, .. }, picture: None }) => Some(url.clone()),
                _ => {
                    st.shown = None;
                    None
                }
            }
        };
        if debug() {
            eprintln!("debug navegador: passo {} mostrado", i + 1);
        }
        match open {
            Some(u) => self.push(Job::Open(u, Some(i))),
            None => self.render(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answers_words_point_at_what_was_read() {
        let w = answer_words("O Corinthians venceu o Botafogo por 4 a 0: três gols de Kaio César e um contra de Arthur Chaves, aos 28'.");
        for x in ["Corinthians", "Botafogo", "Kaio", "César", "Arthur", "Chaves", "28'"] {
            assert!(w.iter().any(|y| y == x.trim_end_matches('\'')), "{x} in {w:?}");
        }
        assert!(!w.iter().any(|y| y == "venceu" || y == "por"));
    }

    #[test]
    fn a_pages_site() {
        assert_eq!(host("https://www.ge.globo.com/futebol/times/corinthians/"), "ge.globo.com");
        assert_eq!(host("http://127.0.0.1:18766/time-kalango"), "127.0.0.1:18766");
    }
}
