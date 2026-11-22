//! KalangoOS's own Help (design "Ajuda", 62932; app id os.kalango.Ajuda; `kalango-shell ajuda [QUESTION]`): a
//! question answered by the local model from the installed manual (share/kalango/manual/*.md, one topic a file with
//! its words and its Settings page) — it works with no internet. "Mostre para mim" opens Settings and shows on the
//! real screen where to click (the rest dims 40%, an accent ring pulses on the target); "Abrir Configurações › …"
//! goes there; "Fazer por mim" hands the question to the agent (its steps go to the Timeline). Getting started,
//! common questions, the whole manual, and a problem report saved on the computer.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{cairo, gdk, gio, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

extern "C" {
    fn gdk_wayland_toplevel_set_application_id(toplevel: *mut gdk::ffi::GdkToplevel, id: *const std::ffi::c_char);
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// A topic of the manual.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Topic {
    pub id: String,
    pub title: String,
    pub words: Vec<String>,
    /// the Settings page it is about ("tela", "rede"…), if any
    pub page: Option<String>,
    pub text: String,
}

/// Where the installed manual is: `KALANGO_MANUAL`, or share/kalango/manual in the system's data folders.
fn manual_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("KALANGO_MANUAL") {
        return Some(d.into());
    }
    glib::system_data_dirs().into_iter().map(|d| d.join("kalango/manual")).find(|d| d.is_dir())
}

pub fn parse(id: &str, text: &str) -> Topic {
    let mut t = Topic { id: id.to_string(), ..Default::default() };
    let body = match text.strip_prefix("---\n").and_then(|r| r.split_once("\n---\n")) {
        Some((head, body)) => {
            for l in head.lines() {
                match l.split_once(':') {
                    Some(("titulo", v)) => t.title = v.trim().to_string(),
                    Some(("palavras", v)) => t.words = v.split(',').map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).collect(),
                    Some(("pagina", v)) => t.page = Some(v.trim().to_string()).filter(|p| !p.is_empty()),
                    _ => {}
                }
            }
            body
        }
        None => text,
    };
    t.text = body.trim().to_string();
    t
}

pub fn manual() -> Vec<Topic> {
    let Some(dir) = manual_dir() else { return Vec::new() };
    let mut v: Vec<Topic> = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .filter_map(|p| Some(parse(&p.file_stem()?.to_string_lossy(), &std::fs::read_to_string(&p).ok()?))).collect();
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
}

fn plain(s: &str) -> String {
    s.to_lowercase().chars().map(|c| match c {
        'á' | 'à' | 'â' | 'ã' => 'a', 'é' | 'ê' => 'e', 'í' => 'i', 'ó' | 'ô' | 'õ' => 'o', 'ú' | 'ü' => 'u', 'ç' => 'c', c => c,
    }).collect()
}

/// The topics that fit a question best (its words in theirs), the best first.
pub fn best(topics: &[Topic], q: &str, n: usize) -> Vec<Topic> {
    let qw: Vec<String> = plain(q).split(|c: char| !c.is_alphanumeric() && c != '-').filter(|w| w.chars().count() >= 3).map(str::to_string).collect();
    let mut scored: Vec<(usize, &Topic)> = topics.iter().map(|t| {
        let hay: Vec<String> = t.words.iter().map(|w| plain(w)).chain([plain(&t.title)]).collect();
        let score = qw.iter().map(|w| hay.iter().filter(|h| h.contains(w.as_str()) || (w.len() >= 5 && w.contains(h.as_str()) && h.len() >= 4)).count()).sum();
        (score, t)
    }).filter(|(s, _)| *s > 0).collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
    scored.into_iter().take(n).map(|(_, t)| t.clone()).collect()
}

/// A Settings page's name as shown ("Tela", "Rede"…).
fn page_name(id: &str) -> String {
    crate::configuracoes::nome_da_pagina(id).unwrap_or(id).to_string()
}

pub struct HelpApp {
    window: gtk::ApplicationWindow,
    entry: gtk::Entry,
    answer: gtk::Box,
    answer_text: gtk::Label,
    show_me: gtk::Button,
    open_page: gtk::Button,
    do_it: gtk::Button,
    topics: RefCell<Vec<Topic>>,
    current: RefCell<Option<(String, Option<String>)>>,
    overlay: RefCell<Option<gtk::Window>>,
    recording: Rc<RefCell<Option<crate::voz::Recording>>>,
    /// Bumped every time the window is hidden: what a dictation started before that is no longer wanted (a
    /// transcription that arrives after the window was closed must not fill the entry nor ask the agent on reopening).
    generation: Rc<Cell<u64>>,
    shell: Weak<Shell>,
}

thread_local! {
    static APP: RefCell<Option<Rc<HelpApp>>> = const { RefCell::new(None) };
}

pub fn open(shell: &Rc<Shell>, question: Option<String>) {
    let a = APP.with(|a| a.borrow().clone()).unwrap_or_else(|| {
        let app = gio::Application::default().and_downcast::<gtk::Application>().expect("app");
        let a = HelpApp::new(&app, shell);
        APP.with(|x| *x.borrow_mut() = Some(a.clone()));
        a
    });
    if !a.window.is_visible() {
        let (w, h) = crate::superficies::tamanho_de_janela(1500, 910);
        a.window.set_default_size(w, h);
    }
    a.window.present();
    *a.topics.borrow_mut() = manual();
    if debug() {
        eprintln!("debug ajuda: manual com {} tópicos ({})", a.topics.borrow().len(), manual_dir().map(|d| d.display().to_string()).unwrap_or_default());
    }
    if let Some(q) = question.filter(|q| !q.trim().is_empty()) {
        a.entry.set_text(&q);
        a.ask(q);
    }
}

/// Opens the Help on one topic of the manual (by its file's name, "05-agentes-permissoes").
pub fn open_topic(shell: &Rc<Shell>, id: &str) {
    open(shell, None);
    if let Some(a) = APP.with(|a| a.borrow().clone()) {
        a.show_topic(id);
    }
}

fn start_card(glyph: &str, color: &str, title: &str, sub: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("aju-cartao");
    b.set_hexpand(true);
    let col = caixa(Orientation::Vertical, 6, "");
    let ic = caixa(Orientation::Horizontal, 0, "aju-cartao-icone");
    let g = ui_icon(glyph, color, 14);
    g.set_hexpand(true);
    ic.append(&g);
    ic.set_halign(Align::Start);
    col.append(&ic);
    let t = rotulo(title, "aju-cartao-titulo");
    t.set_xalign(0.0);
    t.set_margin_top(8);
    col.append(&t);
    let s = rotulo(sub, "aju-cartao-sub");
    s.set_xalign(0.0);
    s.set_wrap(true);
    s.set_max_width_chars(1);
    col.append(&s);
    b.set_child(Some(&col));
    b
}

impl HelpApp {
    fn new(app: &gtk::Application, shell: &Rc<Shell>) -> Rc<HelpApp> {
        let (w, h) = crate::superficies::tamanho_de_janela(1500, 910);
        let window = gtk::ApplicationWindow::builder().application(app).title(tr("Help")).default_width(w).default_height(h).build();
        // (its app id as its name: the state written by `kalango-shell estado` finds it by it)
        window.set_widget_name("os.kalango.Ajuda");
        // (one window for the app, kept: closing hides it and opening shows it again — a closed GTK window shown again
        // could not be closed any more, and it kept text input from the other windows)
        window.set_hide_on_close(true);
        // (closed for a while, the window lets go of its surface and renderer and keeps its widgets: see
        // superficies::release_resources_when_hidden)
        crate::superficies::release_resources_when_hidden(&window, || true);
        window.add_css_class("ed-janela");
        let sem_barra = gtk::Box::new(Orientation::Horizontal, 0);
        sem_barra.set_visible(false);
        window.set_titlebar(Some(&sem_barra));
        let bar = caixa(Orientation::Horizontal, 10, "ed-barra");
        bar.append(&crate::apps::imagem_kalango("ajuda", 24));
        let name = rotulo(tr("Help"), "img-titulo");
        name.set_hexpand(true);
        name.set_xalign(0.0);
        bar.append(&name);
        bar.append(&crate::configuracoes::controles_janela());
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&bar));
        let line = gtk::Separator::new(Orientation::Horizontal);
        line.add_css_class("vid-linha");

        let col = caixa(Orientation::Vertical, 18, "aju-coluna");
        col.set_halign(Align::Center);
        col.set_size_request(-1, -1);
        let title = rotulo(tr("How can I help?"), "aju-titulo");
        col.append(&title);
        // the question
        let field = caixa(Orientation::Horizontal, 8, "aju-campo");
        field.append(&ui_icon("spark", "#A5A1E2", 16));
        let entry = gtk::Entry::new();
        entry.add_css_class("aju-entrada");
        entry.set_hexpand(true);
        entry.set_placeholder_text(Some(tr("Ask anything about the computer: “how do I connect a second monitor?”")));
        field.append(&entry);
        let mic = gtk::Button::new();
        mic.add_css_class("aju-mic");
        mic.set_child(Some(&ui_icon("microfone", "#8E8F99", 14)));
        mic.set_tooltip_text(Some(tr("Ask by voice (click again to stop)")));
        crate::central::posicao_debug_sempre("ajuda microfone", mic.upcast_ref());
        field.append(&mic);
        let send = gtk::Button::new();
        send.add_css_class("aju-enviar");
        send.set_child(Some(&ui_icon("enviar", "#1C1D24", 14)));
        crate::central::posicao_debug_sempre("ajuda enviar", send.upcast_ref());
        field.append(&send);
        col.append(&field);
        // the answer
        let answer = caixa(Orientation::Vertical, 10, "aju-resposta");
        let ah = caixa(Orientation::Horizontal, 8, "");
        ah.append(&ui_icon("spark", "#A5A1E2", 14));
        ah.append(&rotulo(tr("ANSWER"), "dsc-secao"));
        answer.append(&ah);
        let answer_text = rotulo("", "aju-resposta-texto");
        answer_text.set_wrap(true);
        answer_text.set_xalign(0.0);
        answer_text.set_max_width_chars(1);
        answer_text.set_selectable(true);
        answer.append(&answer_text);
        let acts = caixa(Orientation::Horizontal, 8, "");
        let show_me = gtk::Button::new();
        show_me.add_css_class("aju-mostre");
        let sm = caixa(Orientation::Horizontal, 6, "");
        sm.append(&ui_icon("ponteiro", "#1C1D24", 12));
        sm.append(&rotulo(tr("Show me"), ""));
        show_me.set_child(Some(&sm));
        let open_page = gtk::Button::with_label("");
        open_page.add_css_class("aju-abrir");
        let do_it = gtk::Button::new();
        do_it.add_css_class("aju-fazer");
        let di = caixa(Orientation::Horizontal, 6, "");
        di.append(&ui_icon("spark", "#A5A1E2", 12));
        di.append(&rotulo(tr("Do it for me"), ""));
        do_it.set_child(Some(&di));
        for (b, id) in [(&show_me, "mostre"), (&open_page, "abrir"), (&do_it, "fazer")] {
            crate::central::posicao_debug_sempre(&format!("ajuda {id}"), b.upcast_ref());
            acts.append(b);
        }
        answer.append(&acts);
        answer.set_visible(false);
        col.append(&answer);
        // getting started
        let gs = rotulo(tr("GETTING STARTED"), "dsc-secao");
        gs.set_margin_top(8);
        col.append(&gs);
        let cards = caixa(Orientation::Horizontal, 14, "");
        cards.set_homogeneous(true);
        let start = [
            ("spark", "#A5A1E2", tr("Meet the island"), tr("what is happening, always on top"), "12-ilha"),
            ("microfone", "#8ED2C3", tr("Talk to the computer"), tr("hold Super and say what you want"), "13-voz"),
            ("desfazer", "#E5C98C", tr("Undo anything"), tr("everything an agent does can be undone"), "06-desfazer"),
            ("teclado", "#8AB4F8", tr("Essential shortcuts"), tr("Super+/ shows them all"), "14-atalhos"),
        ];
        let mut start_buttons = Vec::new();
        for (g, c, t, s, id) in start {
            let b = start_card(g, c, t, s);
            cards.append(&b);
            start_buttons.push((b, id));
        }
        col.append(&cards);
        // common questions
        let cq = rotulo(tr("COMMON QUESTIONS"), "dsc-secao");
        cq.set_margin_top(8);
        col.append(&cq);
        let qs = caixa(Orientation::Horizontal, 14, "");
        qs.set_homogeneous(true);
        let common = [
            [("wifi", tr("Connect to Wi-Fi or a printer"), "02-wifi"), ("download", tr("Install apps, agents and models"), "04-apps"),
                ("escudo", tr("What the agents can see and do"), "05-agentes-permissoes"), ("relogio", tr("Take the system back to an earlier point"), "07-ponto-anterior")],
            [("layout", tr("Arrange windows and spaces"), "08-janelas-espacos"), ("paleta", tr("Change theme, dock and wallpaper"), "09-aparencia"),
                ("cpu", tr("Run AI models on this computer"), "10-modelos-locais"), ("olho", tr("Accessibility features"), "11-acessibilidade")],
        ];
        let mut common_buttons = Vec::new();
        for column in common {
            let list = caixa(Orientation::Vertical, 0, "aju-lista");
            for (g, t, id) in column {
                let b = gtk::Button::new();
                b.add_css_class("aju-pergunta");
                let r = caixa(Orientation::Horizontal, 12, "");
                r.append(&ui_icon(g, "#8E8F99", 14));
                let l = rotulo(t, "");
                l.set_hexpand(true);
                l.set_xalign(0.0);
                r.append(&l);
                r.append(&ui_icon("chevron", "#8E8F99", 12));
                b.set_child(Some(&r));
                crate::central::posicao_debug_sempre(&format!("ajuda topico {id}"), b.upcast_ref());
                list.append(&b);
                common_buttons.push((b, id));
            }
            qs.append(&list);
        }
        col.append(&qs);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_vexpand(true);
        scroll.set_child(Some(&col));
        // (the column follows the window, up to the design's 1200)
        let meter = gtk::DrawingArea::new();
        meter.set_can_target(false);
        let measured = gtk::Overlay::new();
        measured.set_child(Some(&scroll));
        measured.add_overlay(&meter);
        measured.set_measure_overlay(&meter, false);
        let cw = col.clone();
        meter.connect_resize(move |_, w, _| cw.set_size_request((w - 96).clamp(520, 1200), -1));
        // the foot
        let foot = caixa(Orientation::Horizontal, 10, "aju-rodape");
        foot.append(&ui_icon("escudo", "#8ED2C3", 14));
        let fl = rotulo(tr("Help works with no internet: the answers come from the local model and the installed manual."), "aju-rodape-texto");
        fl.set_hexpand(true);
        fl.set_xalign(0.0);
        fl.set_ellipsize(gtk::pango::EllipsizeMode::End);
        fl.set_max_width_chars(1);
        foot.append(&fl);
        let manual_b = gtk::Button::new();
        manual_b.add_css_class("aju-link");
        let mb = caixa(Orientation::Horizontal, 6, "");
        mb.append(&ui_icon("doc", "#C9CAD1", 12));
        mb.append(&rotulo(tr("Full manual"), ""));
        manual_b.set_child(Some(&mb));
        foot.append(&manual_b);
        // (the community's address is the distribution's to give: shown only when it is set)
        let community = std::env::var("KALANGO_COMUNIDADE_URL").ok().or_else(|| std::fs::read_to_string("/etc/kalango/comunidade").ok())
            .map(|s| s.trim().to_string()).filter(|s| s.starts_with("https://"));
        let comm_b = gtk::Button::new();
        comm_b.add_css_class("aju-link");
        let cb = caixa(Orientation::Horizontal, 6, "");
        cb.append(&ui_icon("usuario", "#C9CAD1", 12));
        cb.append(&rotulo(tr("Community"), ""));
        comm_b.set_child(Some(&cb));
        comm_b.set_visible(community.is_some());
        foot.append(&comm_b);
        let report = gtk::Button::with_label(tr("Report a problem"));
        report.add_css_class("aju-relatar");
        crate::central::posicao_debug_sempre("ajuda relatar", report.upcast_ref());
        foot.append(&report);
        let root = caixa(Orientation::Vertical, 0, "doc-fundo");
        root.append(&handle);
        root.append(&line);
        root.append(&measured);
        root.append(&foot);
        window.set_child(Some(&root));
        let a = Rc::new(HelpApp {
            window, entry, answer, answer_text, show_me, open_page, do_it, topics: RefCell::default(), current: RefCell::default(),
            overlay: RefCell::default(), recording: Rc::default(), generation: Rc::default(), shell: Rc::downgrade(shell),
        });
        a.wire(&send, &mic, &start_buttons, &common_buttons, &manual_b, &comm_b, community, &report);
        a
    }

    #[allow(clippy::too_many_arguments)]
    fn wire(self: &Rc<Self>, send: &gtk::Button, mic: &gtk::Button, start: &[(gtk::Button, &'static str)], common: &[(gtk::Button, &'static str)],
        manual_b: &gtk::Button, comm_b: &gtk::Button, community: Option<String>, report: &gtk::Button) {
        let w = Rc::downgrade(self);
        self.entry.connect_activate(move |e| {
            let t = e.text().trim().to_string();
            if let (Some(a), false) = (w.upgrade(), t.is_empty()) {
                a.ask(t);
            }
        });
        let w = Rc::downgrade(self);
        send.connect_clicked(move |_| {
            if let Some(a) = w.upgrade() {
                let t = a.entry.text().trim().to_string();
                if !t.is_empty() {
                    a.ask(t);
                }
            }
        });
        // the voice: a click starts listening (the person's choice), another stops; the local model writes it down
        let w = Rc::downgrade(self);
        mic.connect_clicked(move |b| {
            let Some(a) = w.upgrade() else { return };
            let rec = a.recording.borrow_mut().take();
            match rec {
                None => match crate::voz::start("ajuda") {
                    Ok(r) => {
                        *a.recording.borrow_mut() = Some(r);
                        b.add_css_class("gravando");
                        a.say(tr("Listening… click the microphone again when you are done."), None);
                        if debug() {
                            eprintln!("debug ajuda: ouvindo");
                        }
                    }
                    Err(e) => a.say(&trf("The microphone could not be used: {error}", &[("error", e)]), None),
                },
                Some(r) => {
                    b.remove_css_class("gravando");
                    a.say(tr("Writing down what you said…"), None);
                    let w2 = Rc::downgrade(&a);
                    let started_in = a.generation.get();
                    crate::lateral::em_segundo_plano(move || crate::voz::finish(r), move |t: Result<String, String>| {
                        let Some(a) = w2.upgrade() else { return };
                        // the window was closed while it was being written down: the result is dropped
                        if !dictation_is_current(&a.generation, started_in) {
                            if debug() {
                                eprintln!("debug ajuda: transcrição descartada (a janela foi fechada)");
                            }
                            // (its "Writing down what you said…" must not stay on the card after reopening)
                            if clear_status(&a.answer, &a.answer_text, tr("Writing down what you said…")) {
                                a.show_me.set_visible(false);
                                a.open_page.set_visible(false);
                                *a.current.borrow_mut() = None;
                            }
                            return;
                        }
                        match t.map(|t| t.trim().to_string()) {
                            Ok(t) if !t.is_empty() => {
                                if debug() {
                                    eprintln!("debug ajuda: ouvido «{t}»");
                                }
                                a.entry.set_text(&t);
                                a.ask(t);
                            }
                            Ok(_) => a.say(tr("Nothing was heard."), None),
                            Err(e) => a.say(&e, None),
                        }
                    });
                }
            }
        });
        // closed while listening: the listening stops and the recording is thrown away (the window is only hidden, and
        // before this the recorder went on behind it, with the microphone open)
        let w = Rc::downgrade(self);
        stop_listening_when_closed(&self.window, &self.recording, &self.generation, mic, move |text| {
            if let Some(a) = w.upgrade() {
                a.say(text, None);
            }
        });
        for (b, id) in start.iter().chain(common.iter()) {
            let (w, id) = (Rc::downgrade(self), *id);
            b.connect_clicked(move |_| {
                if let Some(a) = w.upgrade() {
                    a.show_topic(id);
                }
            });
        }
        let w = Rc::downgrade(self);
        manual_b.connect_clicked(move |_| {
            if let Some(a) = w.upgrade() {
                let all: Vec<String> = a.topics.borrow().iter().map(|t| format!("• {}", t.title)).collect();
                a.say(&format!("{}\n{}", tr("The manual's topics (ask about any of them):"), all.join("\n")), None);
            }
        });
        if let Some(url) = community {
            let w = Rc::downgrade(self);
            comm_b.connect_clicked(move |_| {
                if let Some(a) = w.upgrade() {
                    gtk::UriLauncher::new(&url).launch(Some(&a.window), gio::Cancellable::NONE, |_| {});
                }
            });
        }
        let w = Rc::downgrade(self);
        report.connect_clicked(move |_| {
            if let Some(a) = w.upgrade() {
                a.report();
            }
        });
        type H = Box<dyn Fn(&Rc<HelpApp>)>;
        for (b, f) in [
            (&self.show_me, Box::new(|a: &Rc<HelpApp>| a.show_me()) as H),
            (&self.open_page, Box::new(|a: &Rc<HelpApp>| {
                if let (Some((_, Some(p))), Some(s)) = (a.current.borrow().clone(), a.shell.upgrade()) {
                    s.abrir_configuracoes(Some(&p));
                }
            })),
            (&self.do_it, Box::new(|a: &Rc<HelpApp>| a.do_it())),
        ] {
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(a) = w.upgrade() {
                    f(&a);
                }
            });
        }
        let app_id = |j: &gtk::ApplicationWindow| {
            if let Some(s) = j.surface().filter(|s| s.type_().name().contains("Wayland")) {
                let id = std::ffi::CString::new("os.kalango.Ajuda").unwrap();
                unsafe { gdk_wayland_toplevel_set_application_id(s.as_ptr() as *mut gdk::ffi::GdkToplevel, id.as_ptr()) };
            }
        };
        self.window.connect_realize(move |j| app_id(j));
        self.window.connect_map(move |j| app_id(j));
    }

    /// The answer card: a text, and the page it is about (its buttons).
    fn say(&self, text: &str, page: Option<String>) {
        self.answer_text.set_text(text);
        self.answer.set_visible(true);
        let has_page = page.is_some();
        self.show_me.set_visible(has_page);
        self.open_page.set_visible(has_page);
        if let Some(p) = &page {
            self.open_page.set_label(&trf("Open Settings › {page}", &[("page", page_name(p))]));
        }
        *self.current.borrow_mut() = Some((self.entry.text().to_string(), page));
    }

    fn show_topic(&self, id: &str) {
        let Some(t) = self.topics.borrow().iter().find(|t| t.id == id).cloned() else { return };
        self.entry.set_text(&t.title);
        self.say(&t.text, t.page.clone());
        if debug() {
            eprintln!("debug ajuda: tópico «{}»", t.title);
        }
    }

    /// A question: the manual's topics that fit it, read by the local model (or, if it cannot, the best topic).
    fn ask(self: &Rc<Self>, q: String) {
        let found = best(&self.topics.borrow(), &q, 4);
        self.say(tr("Looking in the manual…"), None);
        if debug() {
            eprintln!("debug ajuda: pergunta «{q}» ({} tópicos: {})", found.len(), found.iter().map(|t| t.id.clone()).collect::<Vec<_>>().join(", "));
        }
        let input = serde_json::json!({"pergunta": q, "topicos": found.iter().map(|t| serde_json::json!({"id": t.id, "titulo": t.title, "texto": t.text, "pagina": t.page})).collect::<Vec<_>>()});
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || ask_agent("ajuda", &input), move |r| {
            let Some(a) = w.upgrade() else { return };
            match r {
                Ok(v) => {
                    let text = v["resposta"].as_str().unwrap_or("").trim().to_string();
                    let page = v["pagina"].as_str().map(str::to_string).filter(|p| !p.is_empty() && crate::configuracoes::nome_da_pagina(p).is_some());
                    if debug() {
                        eprintln!("debug ajuda: resposta ({} caracteres, página {:?})", text.chars().count(), page);
                    }
                    a.say(&text, page);
                }
                // (the model is not there: the manual's own words)
                Err(_) => match found.first() {
                    Some(t) => a.say(&t.text, t.page.clone()),
                    None => a.say(tr("The manual does not cover that yet. “Do it for me” asks the agent."), None),
                },
            }
        });
    }

    fn do_it(&self) {
        let Some((q, _)) = self.current.borrow().clone() else { return };
        if let Some(s) = self.shell.upgrade() {
            s.right_bar().ask_agent(&q);
        }
        if debug() {
            eprintln!("debug ajuda: fazer por mim «{q}»");
        }
    }

    /// "Mostre para mim": Settings opens, and on the real screen the place to click is shown — the rest dims 40%,
    /// an accent ring pulses on it — until it is clicked (the page changes), Esc, or 12 s.
    fn show_me(self: &Rc<Self>) {
        let Some((_, Some(page))) = self.current.borrow().clone() else { return };
        let Some(s) = self.shell.upgrade() else { return };
        s.abrir_configuracoes(None);
        let w = Rc::downgrade(self);
        let tries = std::cell::Cell::new(0);
        glib::timeout_add_local(Duration::from_millis(250), move || {
            let Some(a) = w.upgrade() else { return glib::ControlFlow::Break };
            tries.set(tries.get() + 1);
            let target = crate::shell_atual().and_then(|s| s.configuracoes_atual()).and_then(|c| c.alvo(&page).map(|b| (c.janela.clone(), b)));
            match target.and_then(|(win, b)| screen_rect(&win, &b, "os.kalango.Configuracoes").map(|r| (r, b))) {
                Some((r, b)) => {
                    a.point_at(r, &b, &page);
                    glib::ControlFlow::Break
                }
                None if tries.get() > 20 => glib::ControlFlow::Break,
                None => glib::ControlFlow::Continue,
            }
        });
    }

    fn point_at(self: &Rc<Self>, (x, y, w, h): (f64, f64, f64, f64), target: &gtk::Button, page: &str) {
        if debug() {
            eprintln!("debug ajuda: mostre para mim «{page}» em {} {} ({}×{})", x.round(), y.round(), w.round(), h.round());
        }
        let Some(app) = gio::Application::default().and_downcast::<gtk::Application>() else { return };
        if let Some(o) = self.overlay.borrow_mut().take() {
            o.close();
        }
        let win = gtk::Window::new();
        win.set_application(Some(&app));
        win.init_layer_shell();
        win.set_layer(Layer::Overlay);
        win.set_namespace(Some("kalango-mostre-para-mim"));
        for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            win.set_anchor(e, true);
        }
        win.set_exclusive_zone(-1);
        win.set_keyboard_mode(KeyboardMode::None);
        win.add_css_class("aju-sobreposicao");
        let da = gtk::DrawingArea::new();
        let t0 = glib::monotonic_time();
        let (cx, cy, rad) = (x + w / 2.0, y + h / 2.0, (w.max(h) / 2.0 + 10.0).max(24.0));
        da.set_draw_func(move |_, cr, ww, hh| {
            // everything dimmed 40%, but the target
            cr.set_fill_rule(cairo::FillRule::EvenOdd);
            cr.rectangle(0.0, 0.0, f64::from(ww), f64::from(hh));
            cr.arc(cx, cy, rad, 0.0, std::f64::consts::TAU);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.4);
            let _ = cr.fill();
            // the ring, pulsing (a beat a second)
            let k = ((glib::monotonic_time() - t0) as f64 / 1_000_000.0) % 1.0;
            let (r, g, b) = crate::tema::rgb("#A5A1E2");
            cr.set_source_rgba(r, g, b, 1.0);
            cr.set_line_width(3.0);
            cr.arc(cx, cy, rad, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
            cr.set_source_rgba(r, g, b, 0.6 * (1.0 - k));
            cr.set_line_width(2.0);
            cr.arc(cx, cy, rad + 4.0 + 14.0 * k, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
        });
        da.add_tick_callback(|d, _| {
            d.queue_draw();
            glib::ControlFlow::Continue
        });
        win.set_child(Some(&da));
        // (it takes no clicks: the click goes to the target under it)
        win.connect_realize(|w| {
            if let Some(s) = w.surface() {
                s.set_input_region(&cairo::Region::create());
            }
        });
        win.present();
        *self.overlay.borrow_mut() = Some(win.clone());
        // gone once the target is clicked, or after 12 s
        let (wk, ww) = (Rc::downgrade(self), win.downgrade());
        let id = RefCell::new(None::<glib::SignalHandlerId>);
        let t2 = target.clone();
        let handler = target.connect_clicked(move |_| {
            if let Some(w) = ww.upgrade() {
                w.close();
            }
            if debug() {
                eprintln!("debug ajuda: alvo clicado");
            }
        });
        *id.borrow_mut() = Some(handler);
        glib::timeout_add_local_once(Duration::from_secs(12), move || {
            if let Some(h) = id.borrow_mut().take() {
                t2.disconnect(h);
            }
            if let Some(a) = wk.upgrade() {
                if let Some(o) = a.overlay.borrow_mut().take() {
                    o.close();
                }
            }
        });
    }

    /// "Relatar um problema": what happened (asked), and the system's own facts (no personal files, no logs unless the
    /// person adds them), saved in Documents to send to whoever helps.
    fn report(self: &Rc<Self>) {
        let d = gtk::Window::builder().transient_for(&self.window).modal(true).title(tr("Report a problem")).default_width(520).build();
        d.add_css_class("ed-janela");
        let col = caixa(Orientation::Vertical, 10, "doc-senha");
        let t = rotulo(tr("What happened? (the report is saved on this computer: nothing is sent)"), "doc-cartao-texto");
        t.set_wrap(true);
        t.set_xalign(0.0);
        col.append(&t);
        let view = gtk::TextView::new();
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_size_request(-1, 140);
        view.add_css_class("aju-relato");
        col.append(&view);
        let ok = gtk::Button::with_label(tr("Save the report"));
        ok.add_css_class("doc-acao");
        ok.set_halign(Align::End);
        crate::central::posicao_debug_sempre("ajuda salvar relato", ok.upcast_ref());
        col.append(&ok);
        d.set_child(Some(&col));
        let (w, dd) = (Rc::downgrade(self), d.clone());
        ok.connect_clicked(move |_| {
            let b = view.buffer();
            let what = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
            dd.close();
            let Some(a) = w.upgrade() else { return };
            let facts = system_facts();
            let now = glib::DateTime::now_local().ok().and_then(|t| t.format("%Y-%m-%d %H-%M").ok()).map(|s| s.to_string()).unwrap_or_default();
            let dir = crate::pastas::documentos();
            let _ = std::fs::create_dir_all(&dir);
            let f = dir.join(trf("Problem report {date}.txt", &[("date", now)]));
            let text = format!("{}\n\n{}\n\n{}\n{}\n", tr("KalangoOS · problem report"), what.trim(), tr("About this computer:"), facts);
            match std::fs::write(&f, text) {
                Ok(()) => {
                    if debug() {
                        eprintln!("debug ajuda: relato salvo «{}»", f.display());
                    }
                    a.say(&trf("The report is saved in {path}. Send it to whoever helps you.", &[("path", f.display().to_string())]), None);
                }
                Err(e) => a.say(&e.to_string(), None),
            }
        });
        d.present();
    }
}

/// The system's facts for a report: version, kernel, the graphics, the memory (nothing personal).
fn system_facts() -> String {
    let os = std::fs::read_to_string("/etc/os-release").ok().and_then(|s| s.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string())))
        .unwrap_or_default();
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim().to_string();
    let mem = std::fs::read_to_string("/proc/meminfo").ok().and_then(|s| s.lines().next().map(|l| l.split_whitespace().nth(1).and_then(|k| k.parse::<u64>().ok()).unwrap_or(0)))
        .map(|k| format!("{:.1} GB", k as f64 / 1e6)).unwrap_or_default();
    let cpu = std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|s| s.lines().find_map(|l| l.strip_prefix("model name").map(|v| v.trim_start_matches([' ', '\t', ':']).to_string())))
        .unwrap_or_default();
    format!("- {}: {os}\n- kernel: {kernel}\n- CPU: {cpu}\n- {}: {mem}\n- KalangoOS shell: {}", tr("system"), tr("memory"), env!("CARGO_PKG_VERSION"))
}

/// Where a widget of a window is on the screen (logical px): the window's place from niri, plus its place in it.
fn screen_rect(win: &gtk::ApplicationWindow, w: &impl IsA<gtk::Widget>, app_id: &str) -> Option<(f64, f64, f64, f64)> {
    if !w.is_mapped() {
        return None;
    }
    let b = w.compute_bounds(win)?;
    let v = crate::servicos::niri_pedido(serde_json::json!("Windows")).ok()?;
    let l = v["Windows"].as_array()?.iter().find(|x| x["app_id"] == app_id)?["layout"].clone();
    let x = l["tile_pos_in_workspace_view"][0].as_f64()? + l["window_offset_in_tile"][0].as_f64().unwrap_or(0.0);
    let y = l["tile_pos_in_workspace_view"][1].as_f64()? + l["window_offset_in_tile"][1].as_f64().unwrap_or(0.0);
    Some((x + f64::from(b.x()), y + f64::from(b.y()), f64::from(b.width()), f64::from(b.height())))
}

fn ask_agent(cmd: &str, input: &serde_json::Value) -> Result<serde_json::Value, String> {
    use std::io::Write;
    let mut c = std::process::Command::new(crate::voz::agent_binary()).arg(cmd).stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    c.stdin.take().ok_or("stdin")?.write_all(input.to_string().as_bytes()).map_err(|e| e.to_string())?;
    let o = c.wait_with_output().map_err(|e| e.to_string())?;
    serde_json::from_slice::<serde_json::Value>(&o.stdout).map_err(|_| String::from_utf8_lossy(&o.stderr).trim().lines().last().unwrap_or("").to_string())
}

/// Hides the answer card when it still shows `status` (a transient line of a dictation that is no longer wanted);
/// anything else it shows is left alone. Whether it was cleared.
fn clear_status(answer: &gtk::Box, text: &gtk::Label, status: &str) -> bool {
    if text.text() != status {
        return false;
    }
    text.set_text("");
    answer.set_visible(false);
    true
}

/// Whether a dictation that began when the window was in generation `started_in` is still wanted: the window has not
/// been hidden since.
fn dictation_is_current(generation: &Cell<u64>, started_in: u64) -> bool {
    generation.get() == started_in
}

/// A window that closes while the microphone listens: the recording stops and is discarded (off the UI thread), the
/// microphone's button goes back to normal and `said` tells the person why. Nothing is said when it is not
/// listening. Hiding always bumps `generation`, so a transcription still being written down is dropped when it arrives.
fn stop_listening_when_closed(window: &impl IsA<gtk::Widget>, recording: &Rc<RefCell<Option<crate::voz::Recording>>>, generation: &Rc<Cell<u64>>,
    mic: &gtk::Button, said: impl Fn(&str) + 'static) {
    let (rec, mic, generation) = (Rc::downgrade(recording), mic.downgrade(), generation.clone());
    window.connect_visible_notify(move |win| {
        if win.is_visible() {
            return;
        }
        generation.set(generation.get() + 1);
        let Some(r) = rec.upgrade().and_then(|r| r.borrow_mut().take()) else { return };
        if let Some(m) = mic.upgrade() {
            m.remove_css_class("gravando");
        }
        said(tr("Listening stopped: the window was closed."));
        crate::lateral::em_segundo_plano(move || crate::voz::discard(r), |()| {});
        if debug() {
            eprintln!("debug ajuda: escuta parada ao fechar a janela");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a display of its own (KALANGO_TESTE_GTK=1, as `release_tests` in superficies.rs). GTK may only be used
    /// from the thread that started it (gtk4 `rt.rs`), so this test and `superficies::release_tests` cannot share one
    /// process: run each on its own, one filter per run:
    /// `KALANGO_TESTE_GTK=1 xvfb-run -a cargo test -p kalango-shell --bin kalango-shell -- --test-threads=1 closing_the_window_stops_the_listening`.
    /// Closing the window
    /// while the recorder runs stops it, resets the button and says why; closing it when not listening does nothing.
    #[test]
    fn closing_the_window_stops_the_listening() {
        if std::env::var_os("KALANGO_TESTE_GTK").is_none() || gtk::init().is_err() {
            eprintln!("(sem KALANGO_TESTE_GTK e uma tela: pulado)");
            return;
        }
        let dir = std::env::temp_dir().join(format!("kalango-ajuda-teste-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("gravador.sh");
        std::fs::write(&script, "#!/bin/bash\n: > \"$1\"\ntrap 'exit 0' INT TERM\nwhile :; do sleep 0.1; done\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::env::set_var("KALANGO_GRAVADOR", &script);
        std::env::set_var("XDG_RUNTIME_DIR", &dir);
        let run = |ms: u64| {
            let ctx = glib::MainContext::default();
            let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
            while std::time::Instant::now() < end {
                ctx.iteration(false);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        };
        let recorder_runs = || std::process::Command::new("pgrep").arg("-f").arg(format!("^/bin/bash {} ", script.display()))
            .output().map(|o| o.status.success()).unwrap_or(false);
        let win = gtk::Window::new();
        let mic = gtk::Button::new();
        let recording: Rc<RefCell<Option<crate::voz::Recording>>> = Rc::default();
        let generation: Rc<Cell<u64>> = Rc::default();
        let said = Rc::new(RefCell::new(Vec::<String>::new()));
        let s = said.clone();
        stop_listening_when_closed(&win, &recording, &generation, &mic, move |t| s.borrow_mut().push(t.to_string()));
        // not listening: closing says nothing
        win.set_visible(true);
        run(50);
        win.set_visible(false);
        run(50);
        assert!(said.borrow().is_empty(), "nothing said when it was not listening");
        // (but the window was hidden: a dictation under way from before is no longer wanted)
        assert_eq!(generation.get(), 1);
        assert!(!dictation_is_current(&generation, 0), "a transcription that began before the close is dropped");
        assert!(dictation_is_current(&generation, 1), "one that began after it is kept");
        // listening: closing stops it
        win.set_visible(true);
        *recording.borrow_mut() = Some(crate::voz::start("teste").expect("the stand-in recorder starts"));
        mic.add_css_class("gravando");
        run(300);
        assert!(recorder_runs(), "the recorder runs while listening");
        win.set_visible(false);
        run(1000);
        assert!(recording.borrow().is_none(), "the recording is let go");
        assert!(!mic.has_css_class("gravando"), "the button is reset");
        assert_eq!(*said.borrow(), vec![tr("Listening stopped: the window was closed.").to_string()]);
        assert!(!recorder_runs(), "the recorder is stopped: the microphone is not open behind the closed window");
        assert_eq!(generation.get(), 2);
        // a dropped transcription takes its own "Writing down…" line off the card, and only that line
        let (card, label) = (gtk::Box::new(gtk::Orientation::Vertical, 0), gtk::Label::new(Some(tr("Writing down what you said…"))));
        card.set_visible(true);
        assert!(clear_status(&card, &label, tr("Writing down what you said…")));
        assert!(!card.is_visible() && label.text().is_empty());
        label.set_text("Uma resposta de verdade");
        card.set_visible(true);
        assert!(!clear_status(&card, &label, tr("Writing down what you said…")) && card.is_visible(), "an answer is kept");
        // closed while listening and reopened at once, listening again before the first one's discard has finished: the
        // new recording is its own file, and the discard does not take it away
        win.set_visible(true);
        *recording.borrow_mut() = Some(crate::voz::start("teste").expect("the stand-in recorder starts"));
        let first = recording.borrow().as_ref().unwrap().path().to_path_buf();
        run(200);
        win.set_visible(false);
        win.set_visible(true);
        *recording.borrow_mut() = Some(crate::voz::start("teste").expect("the stand-in recorder starts"));
        let second = recording.borrow().as_ref().unwrap().path().to_path_buf();
        assert_ne!(first, second, "each recording has its own file");
        run(1500);
        assert!(second.exists(), "the discard of the first recording did not delete the new one");
        assert!(!first.exists(), "the first one was discarded");
        let r = recording.borrow_mut().take().unwrap();
        crate::voz::discard(r);
        win.destroy();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn topics_and_search() {
        let t = parse("01-monitor", "---\ntitulo: Conectar um segundo monitor\npalavras: monitor, tela, hdmi\npagina: tela\n---\nLigue o cabo.\n");
        assert_eq!(t.title, "Conectar um segundo monitor");
        assert_eq!(t.page.as_deref(), Some("tela"));
        assert_eq!(t.text, "Ligue o cabo.");
        let w = parse("02-wifi", "---\ntitulo: Conectar ao Wi-Fi\npalavras: wifi, internet, rede\npagina: rede\n---\nClique na rede.");
        let all = vec![t.clone(), w];
        assert_eq!(best(&all, "como eu conecto um segundo monitor?", 1), vec![t]);
        assert_eq!(best(&all, "minha internet caiu", 1)[0].id, "02-wifi");
        assert!(best(&all, "zzz", 1).is_empty());
    }
}
