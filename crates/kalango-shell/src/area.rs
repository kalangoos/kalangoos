//! The desktop's first widgets (design "04 · Área de trabalho"): the clock, the **day summary** (facts of the system
//! turned into sentences by the local model, `kalango-agente resumo`), **active agents** (the agent of the side bar
//! and the requests waiting in kalangod), "Pergunte ao KalangoOS" and "Agora · escolhido pelo agente". Where they go
//! and the other widgets: widgets/.

use crate::conversa::ui_icon;
use crate::servicos;
use crate::superficies::{caixa, estrela, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::Value;
use std::process::Command;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

const WIDGET: i32 = 352;

/// The person's first name (GECOS of the account, else the login).
fn first_name() -> String {
    let user = std::env::var("USER").unwrap_or_default();
    let gecos = std::fs::read_to_string("/etc/passwd").ok().and_then(|p| {
        p.lines().find(|l| l.split(':').next() == Some(user.as_str())).and_then(|l| l.split(':').nth(4).map(str::to_string))
    });
    let full = gecos.map(|g| g.split(',').next().unwrap_or("").trim().to_string()).filter(|g| !g.is_empty()).unwrap_or_else(|| user.clone());
    let first = full.split_whitespace().next().unwrap_or("").to_string();
    let mut c = first.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

pub(crate) fn greeting() -> String {
    let h = glib::DateTime::now_local().map(|d| d.hour()).unwrap_or(12);
    let name = first_name();
    let base = match h {
        5..=11 => tr("Good morning"),
        12..=17 => tr("Good afternoon"),
        _ => tr("Good evening"),
    };
    if name.is_empty() { base.to_string() } else { format!("{base}, {name}") }
}

fn agent_binary() -> std::path::PathBuf {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("kalango-agente"))).filter(|p| p.is_file())
        .unwrap_or_else(|| "kalango-agente".into())
}

fn label_row(text: &str) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 5, "");
    r.append(&estrela(13, "#A5A1E2"));
    r.append(&rotulo(text, "desk-rotulo"));
    r
}

/// The day summary (left column, under the clock).
struct Summary {
    estado: Rc<crate::estado::Estado>,
    root: gtk::Box,
    label: gtk::Label,
    hello: gtk::Label,
    items: gtk::Box,
}

impl Summary {
    fn new(shell: &Rc<Shell>) -> Summary {
        let root = caixa(Orientation::Vertical, 10, "desk-widget");
        root.set_size_request(WIDGET, -1);
        let lr = label_row(tr("DAY SUMMARY"));
        let label = lr.last_child().and_downcast::<gtk::Label>().unwrap();
        root.append(&lr);
        let hello = rotulo(&greeting(), "desk-ola");
        hello.set_xalign(0.0);
        root.append(&hello);
        let items = caixa(Orientation::Vertical, 10, "");
        let wait = rotulo(tr("Preparing the summary…"), "desk-item-fraco");
        wait.set_xalign(0.0);
        items.append(&wait);
        root.append(&items);
        let actions = caixa(Orientation::Horizontal, 8, "");
        actions.set_homogeneous(true);
        // both open the Agent mode of the side bar: pending requests are listed there, and the composer gets focus
        for (text, class) in [(tr("Review pending"), "desk-botao-principal"), (tr("Open conversation"), "desk-botao")] {
            let b = gtk::Button::with_label(text);
            b.add_css_class(class);
            let s = Rc::downgrade(shell);
            b.connect_clicked(move |_| {
                if let Some(s) = s.upgrade() {
                    s.right_bar().abrir(crate::lateral::Modo::Agente);
                }
            });
            actions.append(&b);
        }
        root.append(&actions);
        Summary { estado: shell.estado.clone(), root, label, hello, items }
    }

    fn show(&self, v: &Value) {
        *self.estado.resumo_dia.borrow_mut() = Some(v.clone());
        self.hello.set_text(&greeting());
        self.label.set_text(if v["gerado"] == true { tr("DAY SUMMARY · MADE BY THE AGENT") } else { tr("DAY SUMMARY") });
        while let Some(c) = self.items.first_child() {
            self.items.remove(&c);
        }
        for it in v["itens"].as_array().into_iter().flatten() {
            let row = caixa(Orientation::Horizontal, 7, "");
            let (glyph, color) = match it["tipo"].as_str() {
                Some("feito") => ("check", "#8ED2C3"),
                Some("aviso") => ("alerta", "#E5C98C"),
                Some("agenda") => ("relogio", "#7CC3DF"),
                _ => ("raio", "#8E8F99"),
            };
            let i = ui_icon(glyph, color, 16);
            i.set_valign(Align::Start);
            row.append(&i);
            let t = rotulo(it["texto"].as_str().unwrap_or(""), "desk-item");
            t.set_wrap(true);
            t.set_xalign(0.0);
            t.set_hexpand(true);
            t.set_max_width_chars(44);
            row.append(&t);
            self.items.append(&row);
        }
    }

    /// Asks the engine for the summary (in the background) now and every 30 minutes.
    fn refresh_forever(self: Rc<Self>) {
        let load = {
            let me = Rc::downgrade(&self);
            move || {
                let me = me.clone();
                let bin = agent_binary();
                crate::lateral::em_segundo_plano(move || {
                    Command::new(bin).arg("resumo").output().ok().and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
                }, move |v: Option<Value>| {
                    if let (Some(s), Some(v)) = (me.upgrade(), v) {
                        s.show(&v);
                    }
                });
            }
        };
        // a moment after the session starts, so it does not compete with the login. The timers keep the summary
        // alive while it is on the desktop and end once it is taken off (in "Editar widgets")
        let (first, me) = (load.clone(), self.clone());
        glib::timeout_add_local_once(Duration::from_secs(4), move || {
            if me.root.root().is_some() {
                first();
            }
        });
        glib::timeout_add_local(Duration::from_secs(30 * 60), move || {
            if self.root.root().is_none() {
                return glib::ControlFlow::Break;
            }
            load();
            glib::ControlFlow::Continue
        });
    }
}

/// One row of "Agentes ativos": state dot, name, what it does, progress.
fn agent_row(name: &str, status: &str, dot: &str, progress: Option<f64>) -> gtk::Box {
    let row = caixa(Orientation::Horizontal, 10, "");
    let d = caixa(Orientation::Horizontal, 0, "desk-ponto");
    d.add_css_class(dot);
    d.set_valign(Align::Start);
    d.set_margin_top(if progress.is_some() { 8 } else { 5 });
    row.append(&d);
    let col = caixa(Orientation::Vertical, 4, "");
    col.set_hexpand(true);
    let line = caixa(Orientation::Horizontal, 6, "");
    line.append(&rotulo(name, "desk-agente-nome"));
    let st = rotulo(status, "desk-agente-estado");
    st.set_ellipsize(gtk::pango::EllipsizeMode::End);
    st.set_xalign(0.0);
    st.set_hexpand(true);
    line.append(&st);
    col.append(&line);
    if let Some(p) = progress {
        let bar = gtk::Overlay::new();
        let track = caixa(Orientation::Horizontal, 0, "desk-barra");
        track.set_hexpand(true);
        bar.set_child(Some(&track));
        let fill = caixa(Orientation::Horizontal, 0, "desk-barra-valor");
        fill.add_css_class(dot);
        fill.set_halign(Align::Start);
        fill.set_size_request(((WIDGET - 24) as f64 * p.clamp(0.04, 1.0)) as i32, 3);
        bar.add_overlay(&fill);
        col.append(&bar);
    }
    row.append(&col);
    row
}

/// The clock (URW Gothic 54 in four colours, the date under it), ticking every second while it is on screen.
pub(crate) fn relogio() -> gtk::Widget {
    let relogio = caixa(Orientation::Vertical, 0, "relogio");
    relogio.set_halign(Align::Start);
    relogio.set_valign(Align::Start);
    let linha = |a: &str, b: &str| {
        let l = caixa(Orientation::Horizontal, 0, "");
        l.set_halign(Align::Center);
        let x = rotulo("", "digito");
        x.add_css_class(a);
        let y = rotulo("", "digito");
        y.add_css_class(b);
        l.append(&x);
        l.append(&y);
        (l, x, y)
    };
    let (lh, h1, h2) = linha("d1", "d2");
    let (lm, m1, m2) = linha("d3", "d4");
    let data = rotulo("", "data-relogio");
    data.set_halign(Align::Center);
    relogio.append(&lh);
    relogio.append(&lm);
    relogio.append(&data);
    let tick = move || {
        let agora = glib::DateTime::now_local().unwrap();
        let hh = agora.format("%H").map(|s| s.to_string()).unwrap_or_default();
        let mm = agora.format("%M").map(|s| s.to_string()).unwrap_or_default();
        let c = |s: &str, i: usize| s.chars().nth(i).map(String::from).unwrap_or_default();
        h1.set_text(&c(&hh, 0));
        h2.set_text(&c(&hh, 1));
        m1.set_text(&c(&mm, 0));
        m2.set_text(&c(&mm, 1));
        data.set_text(&agora.format("%a • %-d").map(|s| s.to_uppercase().replace('.', "")).unwrap_or_default());
    };
    tick();
    let w = relogio.downgrade();
    glib::timeout_add_seconds_local(1, move || {
        if w.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        tick();
        glib::ControlFlow::Continue
    });
    relogio.upcast()
}

/// "Resumo do dia" (L).
pub(crate) fn resumo(shell: &Rc<Shell>) -> gtk::Widget {
    let summary = Rc::new(Summary::new(shell));
    summary.clone().refresh_forever();
    summary.root.clone().upcast()
}

/// The summary in the widget gallery: the last one made (the model is not asked there).
pub(crate) fn resumo_previa(shell: &Rc<Shell>) -> gtk::Widget {
    let s = Summary::new(shell);
    let ultimo = shell.estado.resumo_dia.borrow().clone();
    if let Some(v) = ultimo {
        s.show(&v);
    }
    s.root.upcast()
}

/// "Agora" in the widget gallery: what it is for (its suggestions come from the model, on the desktop).
pub(crate) fn agora_previa() -> gtk::Widget {
    let root = caixa(Orientation::Vertical, 10, "desk-widget");
    root.set_size_request(WIDGET, 168);
    root.append(&label_row(tr("NOW · CHOSEN BY THE AGENT")));
    let t = rotulo(tr("What needs you now, in order: events coming up, requests waiting, warnings."), "desk-agora-nota");
    t.set_wrap(true);
    t.set_xalign(0.0);
    t.set_max_width_chars(44);
    root.append(&t);
    root.upcast()
}

/// "Agentes ativos" (M): the agent of the side bar (what it works on, its steps) and every request waiting for
/// approval in kalangod.
pub(crate) fn agentes_ativos(shell: &Rc<Shell>) -> gtk::Widget {
    let box_ = caixa(Orientation::Vertical, 10, "desk-widget");
    box_.set_size_request(WIDGET, 168);
    box_.append(&label_row(tr("ACTIVE AGENTS")));
    let rows = caixa(Orientation::Vertical, 10, "");
    box_.append(&rows);
    let est = shell.estado.clone();
    let render = {
        let rows = rows.downgrade();
        let est = est.clone();
        move |pending: Vec<Value>| {
            let Some(rows) = rows.upgrade() else { return };
            while let Some(c) = rows.first_child() {
                rows.remove(&c);
            }
            match est.agent_task.borrow().as_ref() {
                Some((task, steps)) => {
                    let what = trf("Working: {task}", &[("task", task.clone())]);
                    // grows with every step, never says "done" before it is
                    let p = (*steps as f64 + 1.0) / (*steps as f64 + 3.0);
                    rows.append(&agent_row(tr("Agent"), &what, "estado-verde", Some(p)));
                }
                None => rows.append(&agent_row(tr("Agent"), tr("Ready · Super+G"), "estado-cinza", None)),
            }
            for p in pending {
                let who = p["caller"].as_str().unwrap_or("").split(" (").next().unwrap_or("").to_string();
                rows.append(&agent_row(&who, tr("Waiting for approval"), "estado-ouro", Some(1.0)));
            }
        }
    };
    let render = Rc::new(render);
    let refresh = {
        let (render, vivo) = (render.clone(), rows.downgrade());
        move || {
            if vivo.upgrade().is_none() {
                return;
            }
            let render = render.clone();
            crate::lateral::em_segundo_plano(|| servicos::kalangod_pendentes().unwrap_or_default(), move |p| render(p));
        }
    };
    refresh();
    est.ouvir(refresh);
    box_.upcast()
}

/// "Pergunte ao KalangoOS" (M).
pub(crate) fn pergunte(shell: &Rc<Shell>) -> gtk::Widget {
    ask_widget(shell).upcast()
}

/// "Agora · escolhido pelo agente" (M).
pub(crate) fn agora(shell: &Rc<Shell>) -> gtk::Widget {
    let a = Agora::new(shell);
    a.clone().refresh_forever();
    a.root.clone().upcast()
}

// ---------------------------------------------------------------- "Pergunte ao KalangoOS"

/// The ask field: what is typed (or dictated) goes to the agent in the side bar; the shortcuts ask common things.
fn ask_widget(shell: &Rc<Shell>) -> gtk::Box {
    let root = caixa(Orientation::Vertical, 10, "desk-widget");
    root.set_size_request(WIDGET, 168);
    root.append(&label_row(tr("ASK KALANGOOS")));
    let campo = caixa(Orientation::Horizontal, 10, "desk-pergunta");
    let entrada = gtk::Entry::new();
    entrada.set_hexpand(true);
    entrada.add_css_class("desk-pergunta-campo");
    entrada.set_placeholder_text(Some(tr("What do you want to do?")));
    campo.append(&entrada);
    let mic = gtk::Button::new();
    mic.set_child(Some(&ui_icon("microfone", "#8E8F99", 16)));
    mic.add_css_class("desk-pergunta-mic");
    mic.set_valign(Align::Center);
    mic.set_tooltip_text(Some(tr("Dictate (the local voice model writes it here)")));
    campo.append(&mic);
    let enviar = gtk::Button::new();
    enviar.set_child(Some(&ui_icon("enviar", "#1C1D24", 16)));
    enviar.add_css_class("desk-pergunta-enviar");
    enviar.set_valign(Align::Center);
    enviar.set_tooltip_text(Some(tr("Ask the agent")));
    campo.append(&enviar);
    root.append(&campo);
    let pedir = {
        let s = Rc::downgrade(shell);
        move |texto: String| {
            if let Some(s) = s.upgrade() {
                s.right_bar().ask_agent(&texto);
            }
        }
    };
    {
        let (pedir, e) = (pedir.clone(), entrada.clone());
        let enviar_texto = move || {
            let t = e.text().trim().to_string();
            if !t.is_empty() {
                e.set_text("");
                pedir(t);
            }
        };
        let et = enviar_texto.clone();
        entrada.connect_activate(move |_| et());
        enviar.connect_clicked(move |_| enviar_texto());
    }
    // dictation: the same local voice model as the composer
    {
        let gravando: Rc<RefCell<Option<crate::voz::Recording>>> = Rc::default();
        let e = entrada.clone();
        mic.connect_clicked(move |b| {
            if let Some(rec) = gravando.borrow_mut().take() {
                b.remove_css_class("gravando");
                e.set_placeholder_text(Some(tr("Transcribing…")));
                let e2 = e.clone();
                crate::lateral::em_segundo_plano(move || crate::voz::finish(rec), move |r: Result<String, String>| {
                    e2.set_placeholder_text(Some(tr("What do you want to do?")));
                    if let Ok(t) = r {
                        if !t.trim().is_empty() {
                            e2.set_text(t.trim());
                            e2.set_position(-1);
                        }
                    }
                });
                return;
            }
            if let Ok(rec) = crate::voz::start("area") {
                *gravando.borrow_mut() = Some(rec);
                b.add_css_class("gravando");
                e.set_placeholder_text(Some(tr("Listening… click the microphone to stop")));
            }
        });
    }
    let atalhos = caixa(Orientation::Horizontal, 6, "");
    atalhos.set_halign(Align::Start);
    for (glifo, nome, pedido) in [
        ("doc", tr("Summarize PDF"), ""),
        ("pasta", tr("Organize"), tr("Organize the Downloads folder by type and date. Show me the plan before moving anything.")),
        ("terminal", tr("Explain error"), tr("Look at the system logs and the services that failed, and explain the latest error in plain words.")),
    ] {
        let b = gtk::Button::new();
        let bb = caixa(Orientation::Horizontal, 6, "");
        bb.append(&ui_icon(glifo, "#8E8F99", 16));
        bb.append(&rotulo(nome, "desk-atalho-texto"));
        b.set_child(Some(&bb));
        b.add_css_class("desk-atalho");
        let pedir = pedir.clone();
        if pedido.is_empty() {
            // "Summarize PDF": pick the file first
            b.connect_clicked(move |_| {
                let pedir = pedir.clone();
                let filtros = vec![crate::arquivos::seletor::Filtro::de("PDF", &["application/pdf"])];
                crate::arquivos::seletor::abrir_arquivo("", tr("Pick a PDF to summarize"), Some(tr("Summarize")), filtros, None, move |p| {
                    pedir(trf("Summarize the file {path} in a few points, citing the pages.", &[("path", p.display().to_string())]));
                });
            });
        } else {
            b.connect_clicked(move |_| pedir(pedido.to_string()));
        }
        atalhos.append(&b);
    }
    root.append(&atalhos);
    root
}

// ---------------------------------------------------------------- "Agora · escolhido pelo agente"

/// One suggestion: (title, detail, main action label, what it does).
#[derive(Clone)]
struct Suggestion {
    key: String,
    title: String,
    detail: String,
    action: String,
    run: Rc<dyn Fn()>,
}

/// What may need the person now (events coming up, requests waiting, the summary's warnings); the local model puts
/// it in order and writes the notes (`kalango-agente agora`). "Adiar 10 min" hides one for ten minutes.
struct Agora {
    root: gtk::Box,
    shell: std::rc::Weak<Shell>,
    label: gtk::Label,
    dots: gtk::Box,
    title: gtk::Label,
    note: gtk::Label,
    main: gtk::Button,
    items: RefCell<Vec<Suggestion>>,
    page: std::cell::Cell<usize>,
    snoozed: RefCell<HashMap<String, i64>>,
}

impl Agora {
    fn new(shell: &Rc<Shell>) -> Rc<Agora> {
        let root = caixa(Orientation::Vertical, 10, "desk-widget");
        root.set_size_request(WIDGET, 168);
        let topo = caixa(Orientation::Horizontal, 8, "");
        let lr = label_row(tr("NOW"));
        let label = lr.last_child().and_downcast::<gtk::Label>().unwrap();
        lr.set_hexpand(true);
        topo.append(&lr);
        let dots = caixa(Orientation::Horizontal, 4, "");
        dots.set_valign(Align::Center);
        topo.append(&dots);
        root.append(&topo);
        let title = rotulo("", "desk-agora-titulo");
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_max_width_chars(1);
        title.set_hexpand(true);
        root.append(&title);
        let note = rotulo("", "desk-agora-nota");
        note.set_xalign(0.0);
        note.set_wrap(true);
        note.set_lines(2);
        note.set_ellipsize(gtk::pango::EllipsizeMode::End);
        note.set_max_width_chars(1);
        note.set_hexpand(true);
        root.append(&note);
        let acoes = caixa(Orientation::Horizontal, 8, "");
        acoes.set_halign(Align::Start);
        let main = gtk::Button::with_label("");
        main.add_css_class("desk-botao-principal");
        let adiar = gtk::Button::with_label(tr("Snooze 10 min"));
        adiar.add_css_class("desk-botao");
        acoes.append(&main);
        acoes.append(&adiar);
        root.append(&acoes);
        root.set_visible(false);
        let a = Rc::new(Agora { root, shell: Rc::downgrade(shell), label, dots, title, note, main: main.clone(), items: RefCell::default(), page: std::cell::Cell::new(0), snoozed: RefCell::default() });
        let me = Rc::downgrade(&a);
        main.connect_clicked(move |_| {
            if let Some(a) = me.upgrade() {
                let item = a.items.borrow().get(a.page.get()).cloned();
                if let Some(it) = item {
                    (it.run)();
                }
            }
        });
        let me = Rc::downgrade(&a);
        adiar.connect_clicked(move |_| {
            if let Some(a) = me.upgrade() {
                let key = a.items.borrow().get(a.page.get()).map(|i| i.key.clone());
                if let Some(k) = key {
                    let now = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
                    a.snoozed.borrow_mut().insert(k, now + 600);
                    a.clone().refresh();
                }
            }
        });
        a
    }

    fn show(self: &Rc<Self>) {
        let items = self.items.borrow();
        self.root.set_visible(!items.is_empty());
        while let Some(c) = self.dots.first_child() {
            self.dots.remove(&c);
        }
        let page = self.page.get().min(items.len().saturating_sub(1));
        self.page.set(page);
        for i in 0..items.len().min(4) {
            let d = gtk::Button::new();
            d.add_css_class("desk-pagina");
            if i == page {
                d.add_css_class("atual");
            }
            let me = Rc::downgrade(self);
            d.connect_clicked(move |_| {
                if let Some(a) = me.upgrade() {
                    a.page.set(i);
                    a.show();
                }
            });
            self.dots.append(&d);
        }
        if let Some(it) = items.get(page) {
            self.title.set_text(&it.title);
            self.note.set_text(&it.detail);
            self.note.set_visible(!it.detail.is_empty());
            self.main.set_label(&it.action);
        }
    }

    /// The candidates, from what was read in the background (requests waiting, events coming up with a related file).
    fn candidates(&self, pending: usize, eventos: Vec<(crate::agenda::Event, Option<(std::path::PathBuf, usize)>)>) -> Vec<Suggestion> {
        let Some(shell) = self.shell.upgrade() else { return vec![] };
        let now = glib::DateTime::now_local().ok();
        let now_min = now.as_ref().map(|d| (d.hour() * 60 + d.minute()) as u32).unwrap_or(0);
        let now_unix = now.as_ref().map(|d| d.to_unix()).unwrap_or(0);
        let mut out = Vec::new();
        // events in the next 3 hours, with a note found in the files when there is one
        for (e, nota) in eventos {
            let Some(start) = e.start else { continue };
            if start < now_min {
                continue;
            }
            let falta = start - now_min;
            let title = if falta <= 60 { trf("{event} in {n} min", &[("event", e.title.clone()), ("n", falta.to_string())]) } else { trf("{event} at {time}", &[("event", e.title.clone()), ("time", format!("{:02}:{:02}", start / 60, start % 60))]) };
            let (detail, action, run): (String, String, Rc<dyn Fn()>) = match nota {
                Some((caminho, linha)) => {
                    let nome = caminho.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    (trf("{file} mentions “{event}” (line {line})", &[("file", nome.clone()), ("event", e.title.clone()), ("line", linha.to_string())]),
                     trf("Open {file}", &[("file", nome)]),
                     Rc::new(move || {
                         let uri = gtk::gio::File::for_path(&caminho).uri();
                         let _ = gtk::gio::AppInfo::launch_default_for_uri(&uri, gtk::gdk::Display::default().map(|d| d.app_launch_context()).as_ref());
                     }))
                }
                None => (String::new(), tr("Open calendar").to_string(), Rc::new(|| {
                    let _ = Command::new("gnome-calendar").spawn();
                })),
            };
            out.push(Suggestion { key: format!("agenda:{}:{start}", e.title), title, detail, action, run });
        }
        if pending > 0 {
            let s = self.shell.clone();
            out.push(Suggestion {
                key: "pendentes".into(),
                title: if pending == 1 { tr("1 request waits for your approval").to_string() } else { trf("{n} requests wait for your approval", &[("n", pending.to_string())]) },
                detail: String::new(),
                action: tr("Review").to_string(),
                run: Rc::new(move || {
                    if let Some(s) = s.upgrade() {
                        s.right_bar().abrir(crate::lateral::Modo::Agente);
                    }
                }),
            });
        }
        // the day summary's warnings (a full disk, a failed service…): the agent can help
        if let Some(v) = shell.estado.resumo_dia.borrow().as_ref() {
            for it in v["itens"].as_array().into_iter().flatten().filter(|i| i["tipo"] == "aviso") {
                let t = it["texto"].as_str().unwrap_or("").to_string();
                if t.is_empty() || (pending > 0 && t.contains(&pending.to_string()) && t.to_lowercase().contains("aprova")) {
                    continue;
                }
                let s = self.shell.clone();
                let pedido = trf("{fact} What can I do about it?", &[("fact", t.clone())]);
                out.push(Suggestion {
                    key: format!("aviso:{t}"),
                    title: t,
                    detail: String::new(),
                    action: tr("Ask the agent").to_string(),
                    run: Rc::new(move || {
                        if let Some(s) = s.upgrade() {
                            s.right_bar().ask_agent(&pedido);
                        }
                    }),
                });
            }
        }
        let snoozed = self.snoozed.borrow();
        out.retain(|s| snoozed.get(&s.key).is_none_or(|t| *t <= now_unix));
        out
    }

    /// Reads the candidates in the background, has the local model order them and write the notes.
    fn refresh(self: Rc<Self>) {
        let me = Rc::downgrade(&self);
        let indice = self.shell.upgrade().map(|s| s.comando().indice());
        // off the interface thread: the requests waiting, today's events in the next 3 hours and a file about each
        crate::lateral::em_segundo_plano(move || {
            let pending = servicos::kalangod_pendentes().map(|p| p.len()).unwrap_or(0);
            let agora = glib::DateTime::now_local().map(|d| (d.hour() * 60 + d.minute()) as u32).unwrap_or(0);
            let eventos: Vec<_> = crate::agenda::today().into_iter()
                .filter(|e| e.start.is_some_and(|s| s >= agora && s - agora <= 180))
                .map(|e| {
                    let nota = indice.as_ref().and_then(|i| i.search_content(&e.title, 1).into_iter().next()).map(|(f, l)| (f.path, l));
                    (e, nota)
                }).collect();
            (pending, eventos)
        }, move |(pending, eventos)| {
            let Some(a) = me.upgrade() else { return };
            let cands = a.candidates(pending, eventos);
            if cands.is_empty() {
                a.items.borrow_mut().clear();
                a.show();
                return;
            }
            let dados = Value::Array(cands.iter().map(|c| serde_json::json!({"titulo": c.title, "detalhe": c.detail})).collect());
            let me = Rc::downgrade(&a);
            crate::lateral::em_segundo_plano(move || {
                use std::io::Write;
                let mut c = Command::new(agent_binary()).arg("agora").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null()).spawn().ok()?;
                c.stdin.take()?.write_all(dados.to_string().as_bytes()).ok()?;
                serde_json::from_slice::<Value>(&c.wait_with_output().ok()?.stdout).ok()
            }, move |v: Option<Value>| {
                let Some(a) = me.upgrade() else { return };
                let mut ordered = Vec::new();
                let gerado = v.as_ref().is_some_and(|v| v["gerado"] == true);
                match &v {
                    Some(v) => {
                        let notas: Vec<String> = v["notas"].as_array().into_iter().flatten().map(|n| n.as_str().unwrap_or("").to_string()).collect();
                        for (k, i) in v["ordem"].as_array().into_iter().flatten().filter_map(Value::as_u64).enumerate() {
                            if let Some(c) = cands.get(i as usize) {
                                let mut c = c.clone();
                                if let Some(n) = notas.get(k).filter(|n| !n.is_empty()) {
                                    c.detail = n.clone();
                                }
                                ordered.push(c);
                            }
                        }
                    }
                    None => ordered = cands,
                }
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug agora: {} sugestões{}", ordered.len(), if gerado { " ordenadas pelo modelo local" } else { "" });
                }
                a.label.set_text(if gerado { tr("NOW · CHOSEN BY THE AGENT") } else { tr("NOW") });
                *a.items.borrow_mut() = ordered;
                a.page.set(0);
                a.show();
            });
        });
    }

    /// Now (a little after the summary), every 5 minutes, and when the requests waiting change.
    fn refresh_forever(self: Rc<Self>) {
        // the timers keep "Agora" alive while it is on the desktop and end once it is taken off
        let first = self.clone();
        glib::timeout_add_local_once(Duration::from_secs(8), move || {
            if first.root.root().is_some() {
                first.refresh();
            }
        });
        let every = self.clone();
        glib::timeout_add_local(Duration::from_secs(300), move || {
            if every.root.root().is_none() {
                return glib::ControlFlow::Break;
            }
            every.clone().refresh();
            glib::ControlFlow::Continue
        });
        if let Some(s) = self.shell.upgrade() {
            let me = Rc::downgrade(&self);
            let last = std::cell::Cell::new(s.estado.pendentes.get());
            let est = s.estado.clone();
            s.estado.ouvir(move || {
                if est.pendentes.get() != last.get() {
                    last.set(est.pendentes.get());
                    if let Some(a) = me.upgrade() {
                        a.refresh();
                    }
                }
            });
        }
    }
}
