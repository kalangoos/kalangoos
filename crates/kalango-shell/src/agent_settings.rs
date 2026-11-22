//! Agents › Settings › General, Notifications and Advanced (designs "Ajustes · Geral", "Ajustes · Notificações" and
//! "Ajustes · Avançado"): each tab is a column of cards and a side panel of its own (a preview, where the data is).
//! Every choice is kept in agent_prefs.rs at once and acts where it belongs (the engine, the conversations, the
//! notices, the Editor/Terminal/Monitor suggestions).

use crate::agent_prefs::{self, AgentPrefs};
use crate::configuracoes::{cartao, interruptor, rotulo_cartao};
use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{gio, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use std::rc::Rc;

/// One tab: its cards (scrolling) and its side panel.
pub struct Tab {
    pub body: gtk::ScrolledWindow,
    pub side: gtk::Box,
}

pub struct AgentSettings {
    pub general: Tab,
    pub notifications: Tab,
    pub advanced: Tab,
}

fn left(t: &str, class: &str) -> gtk::Label {
    let l = rotulo(t, class);
    l.set_xalign(0.0);
    l
}

/// An option: its name (and what it means, wrapping) and its control on the right.
fn option_row(title: &str, sub: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
    let t = caixa(Orientation::Vertical, 1, "");
    t.set_hexpand(true);
    t.set_valign(Align::Center);
    t.append(&left(title, "cfg-texto-13"));
    if !sub.is_empty() {
        let l = left(sub, "cfg-sub");
        l.set_wrap(true);
        l.set_max_width_chars(1);
        t.append(&l);
        r.add_css_class("alta");
    }
    r.append(&t);
    control.set_valign(Align::Center);
    r.append(control);
    r
}

/// A slider with its value at its right, as wide as the design's (215); `set` gets each new value.
fn slider(range: (u32, u32), value: u32, debug: &str, set: impl Fn(u32) + 'static) -> gtk::Box {
    let b = caixa(Orientation::Horizontal, 10, "");
    let scale = gtk::Scale::with_range(Orientation::Horizontal, f64::from(range.0), f64::from(range.1), 1.0);
    scale.add_css_class("cfg-escala");
    scale.set_draw_value(false);
    scale.set_size_request(215, -1);
    scale.set_value(f64::from(value));
    crate::central::posicao_debug_sempre(debug, scale.upcast_ref());
    let v = rotulo(&value.to_string(), "cfg-valor-pequeno");
    v.set_width_chars(3);
    b.append(&scale);
    b.append(&v);
    scale.connect_value_changed(move |s| {
        let n = s.value().round() as u32;
        v.set_text(&n.to_string());
        set(n);
    });
    b
}

/// A segmented choice; `on_pick` gets the chosen id.
fn segmented(options: &[(&'static str, &str)], current: &str, on_pick: impl Fn(&'static str) + 'static) -> gtk::Box {
    let seg = caixa(Orientation::Horizontal, 2, "cfg-segmentado");
    seg.add_css_class("escuro");
    seg.set_homogeneous(true);
    let on_pick = Rc::new(on_pick);
    let mut first: Option<gtk::ToggleButton> = None;
    for (id, name) in options {
        let b = gtk::ToggleButton::with_label(name);
        b.add_css_class("cfg-seg");
        if let Some(f) = &first {
            b.set_group(Some(f));
        } else {
            first = Some(b.clone());
        }
        b.set_active(*id == current);
        let (id, f) = (*id, on_pick.clone());
        b.connect_toggled(move |b| {
            if b.is_active() {
                f(id);
            }
        });
        crate::central::posicao_debug_sempre(&format!("ajustes {id}"), b.upcast_ref());
        seg.append(&b);
    }
    seg
}

fn switch(on: bool, name: &str, set: impl Fn(&mut AgentPrefs, bool) + 'static) -> gtk::Switch {
    let s = interruptor();
    s.set_active(on);
    crate::central::posicao_debug_sempre(&format!("ajustes {name}"), s.upcast_ref());
    s.connect_active_notify(move |s| {
        let v = s.is_active();
        agent_prefs::update(|p| set(p, v));
    });
    s
}

fn card(title: &str) -> gtk::Box {
    let c = cartao("");
    c.append(&rotulo_cartao("spark", title));
    c
}

/// A tab's column of cards, scrolling when the window is short (and as wide as the window gives).
fn column() -> (gtk::ScrolledWindow, gtk::Box) {
    let col = caixa(Orientation::Vertical, 12, "");
    col.set_margin_top(16);
    col.set_margin_bottom(16);
    let s = gtk::ScrolledWindow::new();
    s.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    s.set_vexpand(true);
    s.set_child(Some(&col));
    (s, col)
}

/// A side panel with its header (the glyph and the name in capitals).
fn side(glyph: &str, title: &str) -> gtk::Box {
    let p = caixa(Orientation::Vertical, 12, "central-painel");
    let cab = caixa(Orientation::Horizontal, 8, "");
    cab.append(&ui_icon(glyph, "#A5A1E2", 14));
    cab.append(&rotulo(title, "central-painel-agente"));
    p.append(&cab);
    p
}

/// The free space of the system's disk (the preview answers with it): free, total and how full it is.
fn disk() -> (String, String, u64) {
    let f = gio::File::for_path("/");
    let i = f.query_filesystem_info("filesystem::free,filesystem::size", gio::Cancellable::NONE).ok();
    let free = i.as_ref().map(|i| i.attribute_uint64("filesystem::free")).unwrap_or(0);
    let size = i.as_ref().map(|i| i.attribute_uint64("filesystem::size")).unwrap_or(0).max(1);
    (glib::format_size(free).to_string(), glib::format_size(size).to_string(), 100 - free * 100 / size)
}

/// What the agent would answer to "how much free space do I have?" at each length.
pub fn preview_answer(answers: &str, free: &str, total: &str, full: u64) -> String {
    match answers {
        "curtas" => trf("{free} free on the system disk.", &[("free", free.into())]),
        "detalhadas" => trf("You have {free} free on the system disk — it is {full}% full. I looked at the disk the system is on (/), {total} in all. If you want to make room, I can look for the biggest folders and show what could go.",
            &[("free", free.into()), ("full", full.to_string()), ("total", total.into())]),
        _ => trf("You have {free} free on the system disk — it is {full}% full.", &[("free", free.into()), ("full", full.to_string())]),
    }
}

/// How big a folder is (what is in it, a few levels down).
fn folder_size(p: &std::path::Path, depth: u32) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else { return 0 };
    rd.flatten().map(|e| match e.metadata() {
        Ok(m) if m.is_dir() && depth > 0 => folder_size(&e.path(), depth - 1),
        Ok(m) => m.len(),
        Err(_) => 0,
    }).sum()
}

fn home_short(p: &std::path::Path) -> String {
    let home = glib::home_dir().display().to_string();
    p.display().to_string().replacen(&home, "~", 1)
}

impl AgentSettings {
    pub fn new() -> AgentSettings {
        let p = agent_prefs::load();
        AgentSettings { general: general(&p), notifications: notifications(&p), advanced: advanced(&p) }
    }
}

// ------------------------------------------------------------------------------------------------- General

fn general(p: &AgentPrefs) -> Tab {
    let (body, col) = column();
    // the preview first (the answers' choice redraws it)
    let side = side("estrela-n", tr("PREVIEW"));
    let pv = caixa(Orientation::Vertical, 10, "cfg-cartao");
    let q = left(tr("How much free space do I have?"), "aj-previa-pergunta");
    q.set_wrap(true);
    pv.append(&q);
    pv.append(&left(&format!("{} · {} · {}", tr("Agent"), crate::conversa::guess_model(), tr("local")), "cfg-sub"));
    let answer = left("", "aj-previa-resposta");
    answer.set_wrap(true);
    answer.set_max_width_chars(1);
    pv.append(&answer);
    side.append(&pv);
    let note = left(tr("With “Short”, one sentence; with “Detailed”, the agent explains how it got to the answer and suggests the next step."), "cfg-sub");
    note.set_wrap(true);
    note.set_max_width_chars(1);
    side.append(&note);
    let (free, total, full) = disk();
    let draw = {
        let answer = answer.clone();
        move |which: &str| answer.set_text(&preview_answer(which, &free, &total, full))
    };
    draw(&p.answers);

    let c = card(tr("HOW THE AGENT ANSWERS"));
    let seg = segmented(&[("curtas", tr("Short")), ("equilibradas", tr("Balanced answers")), ("detalhadas", tr("Detailed"))], &p.answers, move |id| {
        agent_prefs::update(|p| p.answers = id.into());
        draw(id);
    });
    c.append(&option_row(tr("Answers"), tr("how much the agent writes"), &seg));
    c.append(&option_row(tr("Show the steps open"), tr("every tool the agent uses, in sight in the conversation"),
        &switch(p.steps_open, "passos", |p, v| p.steps_open = v)));
    col.append(&c);

    let c = card(tr("WHEN THE AGENT SUGGESTS"));
    c.append(&option_row(tr("Suggestions while you work"), tr("in the Editor, the Terminal and the Monitor — the local model reads what is open"),
        &switch(p.suggestions, "sugestoes", |p, v| p.suggestions = v)));
    c.append(&option_row(tr("Day summary on the desktop"), tr("what changed, what needs attention, what the AI did"),
        &switch(p.day_summary, "resumo", |p, v| p.day_summary = v)));
    col.append(&c);

    let c = card(tr("WHEN THE AGENTS APP OPENS"));
    let seg = segmented(&[("nova", tr("A new conversation")), ("ultima", tr("Last conversation"))], &p.start_on, |id| {
        agent_prefs::update(|p| p.start_on = id.into());
    });
    c.append(&option_row(tr("Start on"), "", &seg));
    col.append(&c);
    Tab { body, side }
}

// ------------------------------------------------------------------------------------------------- Notifications

fn notifications(p: &AgentPrefs) -> Tab {
    let (body, col) = column();
    let side = side("sino-n", tr("PREVIEW"));
    let notice = caixa(Orientation::Vertical, 6, "cfg-cartao");
    notice.append(&left(tr("Organiser finished"), "aj-aviso-titulo"));
    let nb = left(tr("118 files moved into 4 folders in Downloads."), "cfg-sub");
    nb.set_wrap(true);
    nb.set_max_width_chars(1);
    notice.append(&nb);
    let acts = caixa(Orientation::Horizontal, 8, "");
    acts.append(&rotulo(tr("Undo"), "aj-aviso-acao-principal"));
    acts.append(&rotulo(tr("See what changed"), "aj-aviso-acao"));
    notice.append(&acts);
    side.append(&notice);
    let note = left("", "cfg-sub");
    note.set_wrap(true);
    note.set_max_width_chars(1);
    side.append(&note);
    let draw = {
        let (note, notice) = (note.clone(), notice.clone());
        move |which: &str| {
            note.set_text(match which {
                "lista" => tr("With “Only in the list”, the notice is kept in Notifications without showing on the screen."),
                "nada" => tr("No notice: what the agent did stays in its conversation and in the timeline."),
                _ => tr("This is how a finished task's notice arrives; with “Only in the list”, it is kept in Notifications without showing on the screen."),
            });
            notice.set_opacity(if which == "nada" { 0.4 } else { 1.0 });
        }
    };
    draw(&p.task_notice);

    let c = card(tr("WHEN A TASK FINISHES"));
    let seg = segmented(&[("tela", tr("On the screen")), ("lista", tr("Only in the list")), ("nada", tr("Nothing"))], &p.task_notice, move |id| {
        agent_prefs::update(|p| p.task_notice = id.into());
        draw(id);
    });
    c.append(&option_row(tr("Notice"), tr("what the agent did, with the undo at hand"), &seg));
    col.append(&c);

    let c = card(tr("ROUTINES"));
    let seg = segmented(&[("sempre", tr("Always")), ("falha", tr("Only if it fails"))], &p.routine_notice, |id| {
        agent_prefs::update(|p| p.routine_notice = id.into());
    });
    c.append(&option_row(tr("Notice when a routine runs"), "", &seg));
    col.append(&c);

    let c = card(tr("WHEN THE AGENT NEEDS YOU"));
    c.append(&option_row(tr("Play a sound"), tr("approvals and questions always show on the screen"),
        &switch(p.sound, "som", |p, v| p.sound = v)));
    col.append(&c);

    let c = card(tr("DO NOT DISTURB"));
    c.append(&option_row(tr("The agents' notices wait for Do not disturb"), tr("they go to the list; approvals still come"),
        &switch(p.respect_dnd, "nao-perturbe", |p, v| p.respect_dnd = v)));
    col.append(&c);
    Tab { body, side }
}

// ------------------------------------------------------------------------------------------------- Advanced

fn advanced(p: &AgentPrefs) -> Tab {
    let (body, col) = column();
    let side = side("doc", tr("WHERE THE DATA IS"));
    let list = caixa(Orientation::Vertical, 10, "cfg-cartao");
    let conversas = crate::conversa::conversas_dir();
    let cfg = glib::user_config_dir().join("kalango");
    let memoria = glib::user_data_dir().join("kalango/memoria.json");
    for (name, value) in [
        (tr("Conversations"), format!("{} · {}", home_short(&conversas), glib::format_size(folder_size(&conversas, 2)))),
        (tr("Memory"), home_short(&memoria)),
        (tr("Settings and permissions"), home_short(&cfg)),
        (tr("Record of the actions"), tr("kalangod keeps it (Timeline)").to_string()),
    ] {
        let r = caixa(Orientation::Vertical, 2, "");
        r.append(&left(name, "cfg-sub"));
        let v = left(&value, "cfg-texto-13");
        v.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        v.set_max_width_chars(1);
        r.append(&v);
        list.append(&r);
    }
    side.append(&list);
    let note = left(tr("Everything stays on this computer. Erasing it all at once is in Settings › Privacy."), "cfg-sub");
    note.set_wrap(true);
    note.set_max_width_chars(1);
    side.append(&note);

    let c = card(tr("EACH REQUEST"));
    let row = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
    let t = left(tr("Steps per request"), "cfg-texto-13");
    row.append(&t);
    let scale = gtk::Scale::with_range(Orientation::Horizontal, f64::from(agent_prefs::MIN_STEPS), f64::from(agent_prefs::MAX_STEPS), 1.0);
    scale.add_css_class("cfg-escala");
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.set_value(f64::from(p.steps_per_request));
    crate::central::posicao_debug_sempre("ajustes passos-por-pedido", scale.upcast_ref());
    row.append(&scale);
    let value = rotulo(&p.steps_per_request.to_string(), "cfg-valor-pequeno");
    value.set_width_chars(3);
    row.append(&value);
    scale.connect_value_changed(move |s| {
        let n = s.value().round() as u32;
        value.set_text(&n.to_string());
        agent_prefs::update(|p| p.steps_per_request = n);
    });
    c.append(&row);
    let sub = left(tr("how many actions the agent may take for one request before it stops and tells you"), "cfg-sub");
    sub.set_wrap(true);
    sub.set_max_width_chars(1);
    c.append(&sub);
    col.append(&c);

    // the conversation's memory (the engine's window and summary, docs/CONTEXTO.md)
    let c = card(tr("CONVERSATION MEMORY"));
    c.append(&option_row(tr("Remember the last exchanges whole"), tr("the older ones become a summary; the agent still finds what was said in them"),
        &slider(agent_prefs::KEEP_RANGE, p.keep_exchanges, "ajustes trocas-mantidas", |n| agent_prefs::update(|p| p.keep_exchanges = n))));
    c.append(&option_row(tr("Summarize long conversations every"), tr("exchanges since the last summary · at 0, nothing is summarized and the older ones only leave the window"),
        &slider(agent_prefs::COMPACT_RANGE, p.compact_every, "ajustes resumir-a-cada", |n| agent_prefs::update(|p| p.compact_every = n))));
    col.append(&c);

    let c = card(tr("CONVERSATIONS"));
    let current = match p.keep_days { 90 => "90", 30 => "30", 7 => "7", _ => "sempre" };
    let seg = segmented(&[("sempre", tr("Always")), ("90", tr("90 days")), ("30", tr("30 days")), ("7", tr("7 days"))], current, |id| {
        let days = id.parse().unwrap_or(0);
        agent_prefs::update(|p| p.keep_days = days);
        agent_prefs::prune_conversations(days);
    });
    c.append(&option_row(tr("Keep the conversations"), tr("older ones leave by themselves (pinned ones stay)"), &seg));
    c.append(&option_row(tr("Show the actions' technical details"), tr("what each tool got and gave back, in JSON"),
        &switch(p.technical_details, "detalhes", |p, v| p.technical_details = v)));
    col.append(&c);

    let c = card(tr("THE AGENTS' CONFIGURATION"));
    let buttons = caixa(Orientation::Horizontal, 8, "");
    let export = gtk::Button::new();
    export.add_css_class("central-botao");
    let ec = caixa(Orientation::Horizontal, 6, "");
    ec.append(&ui_icon("baixar", "#E2E2E6", 14));
    ec.append(&rotulo(tr("Export…"), ""));
    export.set_child(Some(&ec));
    let import = gtk::Button::new();
    import.add_css_class("central-botao");
    let ic = caixa(Orientation::Horizontal, 6, "");
    ic.append(&ui_icon("baixar", "#E2E2E6", 14));
    ic.append(&rotulo(tr("Import…"), ""));
    import.set_child(Some(&ic));
    buttons.append(&export);
    buttons.append(&import);
    c.append(&option_row(tr("Export or import"), tr("permissions, connectors, routines and these settings, in one file"), &buttons));
    let result = left("", "cfg-sub");
    result.set_wrap(true);
    result.set_visible(false);
    c.append(&result);
    col.append(&c);
    let r = result.clone();
    export.connect_clicked(move |_| {
        let r = r.clone();
        crate::arquivos::seletor::salvar_arquivo(tr("Agents"), tr("Export the agents' configuration"), "agentes-kalango.json", None, move |path| {
            let text = serde_json::to_string_pretty(&agent_prefs::export_bundle()).unwrap_or_default();
            r.set_visible(true);
            match std::fs::write(&path, text) {
                Ok(()) => {
                    r.set_text(&trf("Saved in {path}.", &[("path", home_short(&path))]));
                    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                        eprintln!("debug agente-ajustes: exportado em {}", path.display());
                    }
                }
                Err(e) => r.set_text(&e.to_string()),
            }
        });
    });
    let r = result;
    import.connect_clicked(move |_| {
        let r = r.clone();
        let filtros = vec![crate::arquivos::seletor::Filtro::de(tr("Agents' configuration"), &["*.json"])];
        crate::arquivos::seletor::abrir_arquivo(tr("Agents"), tr("Import the agents' configuration"), None, filtros, None, move |path| {
            r.set_visible(true);
            let v = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()).unwrap_or_default();
            match agent_prefs::import_bundle(&v) {
                Ok(parts) => {
                    r.set_text(tr("Imported: the new configuration counts from the next request (reopen this window to see the settings)."));
                    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                        eprintln!("debug agente-ajustes: importado de {} ({})", path.display(), parts.join(", "));
                    }
                }
                Err(e) => r.set_text(&e),
            }
        });
    });
    Tab { body, side }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_preview_follows_the_length() {
        let s = super::preview_answer("curtas", "83 GB", "1 TB", 92);
        let m = super::preview_answer("equilibradas", "83 GB", "1 TB", 92);
        let l = super::preview_answer("detalhadas", "83 GB", "1 TB", 92);
        assert!(s.len() < m.len() && m.len() < l.len());
        assert!(s.contains("83 GB") && m.contains("92%") && l.contains("1 TB"));
    }
}
