//! An answer that becomes an interface, for the memory ("o que está usando a memória?"): what uses it now, by program
//! (the person's processes, summed by name), the local AI models among them — they can pause until the next request —
//! and the Monitor to follow. Measured here, from /proc.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use std::rc::{Rc, Weak};

pub fn is_question(q: &str) -> bool {
    let q = q.to_lowercase();
    let mem = ["memória", "memoria", "memory", " ram"].iter().any(|w| q.contains(w)) || q.starts_with("ram");
    let uso = ["usando", "ocupando", "ocupa", "using", "cheia", "full", "lento", "slow", "consumindo", "gastando"].iter().any(|w| q.contains(w));
    mem && uso
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub total: u64,
    pub avail: u64,
    /// (program, bytes, processes)
    pub programs: Vec<(String, u64, usize)>,
    /// what the local model servers hold
    pub models: u64,
}

/// What uses the memory now (the person's processes; off the interface thread).
pub fn measure() -> Report {
    let s = crate::monitor_dados::snap(false);
    let uid = unsafe { getuid() };
    let mut by: std::collections::BTreeMap<String, (u64, usize)> = Default::default();
    let mut models = 0u64;
    for p in s.procs.iter().filter(|p| p.uid == uid) {
        if p.name == MODELS {
            models += p.rss;
        }
        let e = by.entry(p.name.clone()).or_default();
        e.0 += p.rss;
        e.1 += 1;
    }
    let mut programs: Vec<(String, u64, usize)> = by.into_iter().map(|(n, (b, k))| (n, b, k)).collect();
    programs.sort_by_key(|p| std::cmp::Reverse(p.1));
    programs.truncate(5);
    Report { total: s.mem_total, avail: s.mem_avail, programs, models }
}

extern "C" {
    fn getuid() -> u32;
}

/// The local model servers' process.
const MODELS: &str = "llama-server";

/// (executable's file name, the app's name) of the installed apps.
fn app_names() -> Vec<(String, String)> {
    use gtk::gio::prelude::AppInfoExt;
    gtk::gio::AppInfo::all()
        .into_iter()
        .filter_map(|a| {
            let base = crate::apps::executable_name(&a)?;
            (!base.is_empty() && !["env", "sh", "bash", "flatpak", "snap"].contains(&base.as_str())).then(|| (base, a.display_name().to_string()))
        })
        .collect()
}

/// A process's name as people know it: its app's (the same executable, or one whose name holds it), else capitalised.
fn display_name(process: &str, apps: &[(String, String)]) -> String {
    // the system's own: the shell runs the desktop and its apps
    if process == "kalango-shell" {
        return "KalangoOS".into();
    }
    // (one executable behind several apps says nothing of which one this is)
    let same: Vec<&(String, String)> = apps.iter().filter(|(e, _)| e == process).collect();
    if let [(_, n)] = same.as_slice() {
        return n.clone();
    }
    if same.len() > 1 {
        let mut c = process.chars();
        return c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
    }
    if process.chars().count() >= 4 {
        if let Some((_, n)) = apps.iter().find(|(e, _)| e.split(['-', '_', '.']).any(|part| part == process)) {
            return n.clone();
        }
    }
    let mut c = process.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn size(b: u64) -> String {
    glib::format_size(b).to_string()
}

pub fn headline(r: &Report) -> String {
    let used = r.total.saturating_sub(r.avail);
    let pct = if r.total > 0 { (used as f64 / r.total as f64 * 100.0).round() as u64 } else { 0 };
    match r.programs.first() {
        Some((n, b, _)) if n == MODELS => trf("The local AI models use the most memory: {size} of the {pct}% in use", &[("size", size(*b)), ("pct", pct.to_string())]),
        Some((n, b, _)) => trf("{program} uses the most memory: {size} of the {pct}% in use", &[("program", n.clone()), ("size", size(*b)), ("pct", pct.to_string())]),
        None => trf("{pct}% of the memory is in use", &[("pct", pct.to_string())]),
    }
}

pub fn widget(r: &Report, shell: Weak<crate::Shell>, close: Rc<dyn Fn()>) -> gtk::Widget {
    // by the name people know: the app's (from its .desktop), not the process's (looked up here, on the interface's
    // thread — GIO's app list is not to be read from the measuring thread)
    let mut r = r.clone();
    let apps = app_names();
    for p in r.programs.iter_mut() {
        if p.0 != MODELS {
            p.0 = display_name(&p.0, &apps);
        }
    }
    let r = &r;
    let card = caixa(Orientation::Vertical, 12, "cmd-ui");
    let head = caixa(Orientation::Horizontal, 6, "");
    head.append(&ui_icon("spark", "#A5A1E2", 14));
    let h = rotulo(tr("ANSWER · INTERFACE MADE NOW"), "cmd-secao-r");
    h.set_hexpand(true);
    h.set_xalign(0.0);
    head.append(&h);
    head.append(&rotulo(&trf("{free} free of {total}", &[("free", size(r.avail)), ("total", size(r.total))]), "cmd-ui-livre"));
    card.append(&head);
    let t = rotulo(&headline(r), "cmd-ui-titulo");
    t.set_xalign(0.0);
    t.set_wrap(true);
    card.append(&t);
    let bar = gtk::LevelBar::for_interval(0.0, 1.0);
    bar.add_css_class("plano-barra");
    let usado = if r.total > 0 { r.total.saturating_sub(r.avail) as f64 / r.total as f64 } else { 0.0 };
    card.append(&bar);
    let bw = bar.downgrade();
    crate::disk_answer::grow(&bar, move |p| {
        if let Some(b) = bw.upgrade() {
            b.set_value(usado * p);
        }
    });
    let rows = caixa(Orientation::Vertical, 4, "");
    let mut linhas: Vec<gtk::Widget> = Vec::new();
    for (i, (n, b, k)) in r.programs.iter().enumerate() {
        let row = caixa(Orientation::Horizontal, 10, "cmd-ui-linha");
        let dot = caixa(Orientation::Horizontal, 0, "cmd-ui-ponto");
        dot.set_size_request(7, 7);
        dot.set_valign(Align::Center);
        dot.set_hexpand(false);
        dot.add_css_class(["roxo", "verde", "azul", "ouro", "roxo"][i % 5]);
        row.append(&dot);
        let nome = if n == MODELS { tr("Local AI models").to_string() } else { n.clone() };
        let l = rotulo(&if *k > 1 { trf("{program} ({n} processes)", &[("program", nome), ("n", k.to_string())]) } else { nome }, "cmd-ui-nome");
        l.set_hexpand(true);
        l.set_xalign(0.0);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        row.append(&l);
        row.append(&rotulo(&size(*b), "cmd-ui-tamanho"));
        let ver = gtk::Button::with_label(tr("See"));
        ver.add_css_class("cmd-ui-acao");
        ver.set_valign(Align::Center);
        let (s, c) = (shell.clone(), close.clone());
        ver.connect_clicked(move |_| {
            c();
            if let Some(s) = s.upgrade() {
                crate::monitor::open(&s, Some("processos"));
            }
        });
        row.append(&ver);
        linhas.push(row.upcast());
    }
    crate::disk_answer::cascade(&rows, linhas);
    card.append(&rows);
    // the local models can let go of their memory: they start again with the next request
    if r.models > 0 {
        let sug = caixa(Orientation::Horizontal, 10, "cmd-ui-sugestao");
        sug.append(&ui_icon("spark", "#A5A1E2", 16));
        let col = caixa(Orientation::Vertical, 2, "");
        col.set_hexpand(true);
        let a = rotulo(&trf("The local models hold {size}", &[("size", size(r.models))]), "cmd-ui-nome");
        a.set_xalign(0.0);
        col.append(&a);
        let d = rotulo(tr("paused, they let go of it and start again with the next request"), "cmd-ui-sub");
        d.set_xalign(0.0);
        d.set_wrap(true);
        col.append(&d);
        sug.append(&col);
        let b = gtk::Button::with_label(tr("Pause the models"));
        b.add_css_class("cmd-ui-liberar");
        b.set_valign(Align::Center);
        b.connect_clicked(|b| {
            let n = crate::configuracoes_agentes::stop_model_servers(true);
            b.set_label(&trf("{n} paused", &[("n", n.to_string())]));
            b.set_sensitive(false);
        });
        crate::central::posicao_debug("cmd ui pausar modelos", b.upcast_ref());
        sug.append(&b);
        card.append(&sug);
    }
    let pe = caixa(Orientation::Horizontal, 10, "");
    pe.append(&crate::disk_answer::pin_button(shell.clone(), "memoria", crate::widgets::Tam::P));
    let mon = gtk::Button::with_label(tr("Open in the Monitor"));
    mon.add_css_class("cmd-ui-link");
    let (sh, c) = (shell.clone(), close.clone());
    mon.connect_clicked(move |_| {
        c();
        if let Some(s) = sh.upgrade() {
            crate::monitor::open(&s, Some("visao"));
        }
    });
    pe.append(&mon);
    let esp = caixa(Orientation::Horizontal, 0, "");
    esp.set_hexpand(true);
    pe.append(&esp);
    pe.append(&ui_icon("escudo", "#8ED2C3", 13));
    pe.append(&rotulo(tr("measured here · nothing left the computer"), "cmd-ui-sub"));
    card.append(&pe);
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug comando: interface da memória ({} · {} programas)", headline(r), r.programs.len());
    }
    card.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_question() {
        assert!(is_question("o que está usando a memória?"));
        assert!(is_question("what is using my memory"));
        assert!(is_question("a ram está cheia"));
        assert!(!is_question("memória do agente"));
    }

    #[test]
    fn names_people_know() {
        let apps = vec![("google-chrome-stable".to_string(), "Google Chrome".to_string()), ("code".to_string(), "Visual Studio Code".to_string())];
        assert_eq!(display_name("chrome", &apps), "Google Chrome");
        assert_eq!(display_name("code", &apps), "Visual Studio Code");
        assert_eq!(display_name("claude", &apps), "Claude");
        // short names are not guessed inside others
        assert_eq!(display_name("gc", &apps), "Gc");
        // several apps, one program: none of them is named
        let apps = vec![("runner".to_string(), "Images".to_string()), ("runner".to_string(), "Music".to_string())];
        assert_eq!(display_name("runner", &apps), "Runner");
        assert_eq!(display_name("kalango-shell", &apps), "KalangoOS");
    }
}
