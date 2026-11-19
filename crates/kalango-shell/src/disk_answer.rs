//! An answer that becomes an interface (design "Barra de comando · 3 resposta que vira interface"): asked what fills
//! the disk, the command bar answers with the system's own parts — a headline, a bar of what takes the room (videos,
//! AI models, projects, the rest), each with what can be done about it, and the room the build caches take that can
//! go without losing anything of the person's. Measured here, from the disk itself.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

/// A question about what fills the disk ("o que está ocupando meu disco?", "espaço em disco", "what is using my disk").
pub fn is_question(q: &str) -> bool {
    let q = q.to_lowercase();
    let disco = ["disco", "disk", "armazenamento", "storage", "hd ", "ssd"].iter().any(|w| q.contains(w));
    let espaco = ["ocupa", "ocupando", "espaço", "espaco", "space", "using", "cheio", "full", "liberar", "free up"].iter().any(|w| q.contains(w));
    disco && espaco || q.contains("espaço em disco") || q.contains("disk space")
}

/// What takes the room.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub total: u64,
    pub free: u64,
    /// (key, name, bytes, the folder to show)
    pub parts: Vec<(&'static str, String, u64, Option<PathBuf>)>,
    /// Cargo's build folders (they say so with a CACHEDIR.TAG): all can be built again
    pub builds: Vec<PathBuf>,
    pub builds_size: u64,
}

/// The person's folders (the test's area in the session test).
fn home() -> PathBuf {
    std::env::var_os("KALANGO_AREA_TESTE").map(PathBuf::from).unwrap_or_else(glib::home_dir)
}

/// Bytes of some paths (du, apparent sizes off: what they take on the disk).
fn du(paths: &[PathBuf]) -> u64 {
    let existing: Vec<&PathBuf> = paths.iter().filter(|p| p.exists()).collect();
    if existing.is_empty() {
        return 0;
    }
    let o = std::process::Command::new("timeout").arg("20").arg("du").arg("-sbc").args(&existing).stderr(std::process::Stdio::null()).output();
    o.ok().and_then(|o| String::from_utf8_lossy(&o.stdout).lines().last().and_then(|l| l.split_whitespace().next()?.parse().ok())).unwrap_or(0)
}

/// Folders under `base` (to `depth` levels, hidden ones left out) holding a `.git`: the person's projects.
fn projects(base: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(base.to_path_buf(), 0usize)];
    while let Some((d, n)) = stack.pop() {
        if d.join(".git").exists() {
            out.push(d);
            continue;
        }
        if n >= depth {
            continue;
        }
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() && !name.starts_with('.') && !matches!(name.as_str(), "node_modules" | "target" | "snap") {
                stack.push((p, n + 1));
            }
        }
    }
    out
}

/// Cargo's build folders inside a project (`target` with its CACHEDIR.TAG), a few levels down.
fn builds_in(project: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(project.to_path_buf(), 0usize)];
    while let Some((d, n)) = stack.pop() {
        let t = d.join("target");
        if t.join("CACHEDIR.TAG").is_file() {
            out.push(t);
        }
        if n >= 3 {
            continue;
        }
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.path().is_dir() && !name.starts_with('.') && !matches!(name.as_str(), "target" | "node_modules") {
                stack.push((e.path(), n + 1));
            }
        }
    }
    out
}

/// The disk's room and what takes it (off the interface thread: it reads the folders).
pub fn measure() -> Report {
    let h = home();
    // the filesystem the person's folders are on
    let (total, free) = std::process::Command::new("df").args(["-B1", "--output=size,avail"]).arg(&h).output().ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().nth(1).map(|l| {
            let v: Vec<u64> = l.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            (v.first().copied().unwrap_or(0), v.get(1).copied().unwrap_or(0))
        })).unwrap_or((0, 0));
    let videos = crate::pastas::especial(glib::UserDirectory::Videos, "Vídeos");
    let modelos: Vec<PathBuf> = [glib::user_data_dir().join("kalango/modelos"), h.join(".ollama"), glib::user_cache_dir().join("huggingface"), h.join(".lmstudio/models")]
        .into_iter().filter(|p| p.starts_with(&h) || std::env::var_os("KALANGO_AREA_TESTE").is_none()).collect();
    let projs = projects(&h, 4);
    let builds: Vec<PathBuf> = projs.iter().flat_map(|p| builds_in(p)).collect();
    let v = du(&[videos.clone()]);
    let m = du(&modelos);
    let p = du(&projs);
    let b = du(&builds);
    let used = total.saturating_sub(free);
    let others = used.saturating_sub(v + m + p);
    let parts = vec![
        ("videos", tr("Videos").to_string(), v, Some(videos)),
        ("modelos", tr("AI models").to_string(), m, None),
        ("projetos", tr("Projects").to_string(), p, projs.first().and_then(|x| x.parent().map(Path::to_path_buf))),
        ("outros", tr("Other").to_string(), others, Some(h)),
    ];
    Report { total, free, parts, builds, builds_size: b }
}

/// "Vídeos e modelos de IA ocupam 72% do seu disco": the two biggest parts and their share.
pub fn headline(r: &Report) -> String {
    let mut ps: Vec<&(&str, String, u64, Option<PathBuf>)> = r.parts.iter().filter(|p| p.0 != "outros" && p.2 > 0).collect();
    ps.sort_by_key(|p| std::cmp::Reverse(p.2));
    let pct = |b: u64| if r.total > 0 { (b as f64 / r.total as f64 * 100.0).round() as u64 } else { 0 };
    // the system and what lies outside the person's folders take most of it: said so, not "0%"
    let others = r.parts.iter().find(|p| p.0 == "outros").map(|p| p.2).unwrap_or(0);
    if ps.first().is_none_or(|a| others > a.2 + ps.get(1).map(|b| b.2).unwrap_or(0)) && others > 0 {
        return trf("The system and other files take {p}% of your disk", &[("p", pct(others).to_string())]);
    }
    match ps.as_slice() {
        [a, b, ..] => trf("{a} and {b} take {p}% of your disk", &[("a", a.1.clone()), ("b", b.1.to_lowercase()), ("p", pct(a.2 + b.2).to_string())]),
        [a] => trf("{a} take {p}% of your disk", &[("a", a.1.clone()), ("p", pct(a.2).to_string())]),
        [] => trf("{p}% of your disk is in use", &[("p", pct(r.total.saturating_sub(r.free)).to_string())]),
    }
}

fn size(b: u64) -> String {
    glib::format_size(b).to_string()
}

/// Grows 0 → 1 with the slow spring (design: the answer's bar "cresce da esquerda (mola/lenta)"), calling `on` each
/// frame; at once when motion is reduced.
pub(crate) fn grow(w: &impl IsA<gtk::Widget>, on: impl Fn(f64) + 'static) {
    use kalango_movimento as mv;
    if mv::reduzido() {
        on(1.0);
        return;
    }
    let mola = std::cell::RefCell::new(mv::Mola::new(0.0));
    mola.borrow_mut().ir(1.0, mv::LENTA);
    on(0.0);
    let ultimo: Cell<Option<i64>> = Cell::new(None);
    w.add_tick_callback(move |_, clock| {
        let t = clock.frame_time();
        let dt = ultimo.replace(Some(t)).map(|p| (t - p) as f64 / 1e6).unwrap_or(0.0).min(0.05);
        let mut m = mola.borrow_mut();
        m.passo(dt);
        on(m.x.clamp(0.0, 1.0));
        if m.parada() { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
    });
}

/// The answer's rows, each wrapped to come in one after the other (the design's cascade).
pub(crate) fn cascade(rows: &gtk::Box, items: Vec<gtk::Widget>) {
    let anims: Vec<kalango_movimento::Animavel> = items.iter().map(|w| {
        let a = kalango_movimento::Animavel::new(w);
        rows.append(&a);
        a
    }).collect();
    kalango_movimento::cascata(&anims, 6.0);
}

/// "+ Fixar como widget": the answer kept on the desktop as its widget (Ctrl+P in the command bar clicks it).
pub(crate) fn pin_button(shell: Weak<crate::Shell>, kind: &'static str, tam: crate::widgets::Tam) -> gtk::Button {
    let fixar = gtk::Button::new();
    fixar.add_css_class("cmd-ui-acao");
    fixar.add_css_class("cmd-ui-fixar");
    let fc = caixa(Orientation::Horizontal, 6, "");
    fc.append(&ui_icon("mais", "#C9CAD1", 12));
    let fl = rotulo(tr("Pin as a widget"), "");
    fc.append(&fl);
    fixar.set_child(Some(&fc));
    fixar.connect_clicked(move |b| {
        let ok = shell.upgrade().and_then(|s| s.area_de_trabalho()).is_some_and(|a| a.adicionar(kind, tam));
        fl.set_text(if ok { tr("On the desktop") } else { tr("No room on the desktop") });
        b.set_sensitive(false);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug comando: fixado como widget {kind} ({})", if ok { "na área" } else { "sem lugar" });
        }
    });
    crate::central::posicao_debug("cmd ui fixar", fixar.upcast_ref());
    fixar
}

/// The interface: headline, the bar, the parts with their actions, what can go, and where to follow.
pub fn widget(r: &Report, shell: Weak<crate::Shell>, close: Rc<dyn Fn()>) -> gtk::Widget {
    let card = caixa(Orientation::Vertical, 12, "cmd-ui");
    let head = caixa(Orientation::Horizontal, 6, "");
    head.append(&ui_icon("spark", "#A5A1E2", 14));
    let h = rotulo(tr("ANSWER · INTERFACE MADE NOW"), "cmd-secao-r");
    h.set_hexpand(true);
    h.set_xalign(0.0);
    head.append(&h);
    head.append(&rotulo(&trf("{free} free of {total}", &[("free", size(r.free)), ("total", size(r.total))]), "cmd-ui-livre"));
    card.append(&head);
    let t = rotulo(&headline(r), "cmd-ui-titulo");
    t.set_xalign(0.0);
    t.set_wrap(true);
    card.append(&t);
    // the bar: each part's share of what is used
    let cores = ["#A5A1E2", "#8ED2C3", "#7CC3DF", "#E5C98C"];
    let bar = gtk::DrawingArea::new();
    bar.set_content_height(8);
    bar.set_hexpand(true);
    let partes: Vec<u64> = r.parts.iter().map(|p| p.2).collect();
    let crescer: Rc<Cell<f64>> = Rc::new(Cell::new(0.0));
    let cr2 = crescer.clone();
    bar.set_draw_func(move |_, cr, w, h| {
        let tot: u64 = partes.iter().sum::<u64>().max(1);
        let (w, h) = (f64::from(w), f64::from(h));
        // (growing from the left)
        cr.rectangle(0.0, 0.0, w * cr2.get(), h);
        cr.clip();
        let mut x = 0.0;
        for (i, p) in partes.iter().enumerate() {
            let pw = w * (*p as f64 / tot as f64) - 3.0;
            if pw <= 0.0 {
                continue;
            }
            let (rr, g, b) = crate::tema::rgb(cores[i % cores.len()]);
            cr.set_source_rgb(rr, g, b);
            cr.rectangle(x, 0.0, pw, h);
            let _ = cr.fill();
            x += pw + 3.0;
        }
    });
    card.append(&bar);
    let bw = bar.downgrade();
    grow(&bar, move |p| {
        crescer.set(p);
        if let Some(b) = bw.upgrade() {
            b.queue_draw();
        }
    });
    // the parts, each with what can be done about it
    let liberar_once: Rc<Cell<bool>> = Rc::default();
    let rows = caixa(Orientation::Vertical, 4, "");
    let mut linhas: Vec<gtk::Widget> = Vec::new();
    for (i, (key, nome, bytes, pasta)) in r.parts.iter().enumerate() {
        let row = caixa(Orientation::Horizontal, 10, "cmd-ui-linha");
        let dot = caixa(Orientation::Horizontal, 0, "cmd-ui-ponto");
        dot.set_size_request(7, 7);
        dot.set_valign(Align::Center);
        dot.set_hexpand(false);
        dot.add_css_class(["roxo", "verde", "azul", "ouro"][i % 4]);
        row.append(&dot);
        let n = rotulo(nome, "cmd-ui-nome");
        n.set_hexpand(true);
        n.set_xalign(0.0);
        row.append(&n);
        row.append(&rotulo(&size(*bytes), "cmd-ui-tamanho"));
        let acao = |t: &str| {
            let b = gtk::Button::with_label(t);
            b.add_css_class("cmd-ui-acao");
            b.set_valign(Align::Center);
            b
        };
        let b = match *key {
            "videos" => {
                let (s, p, c) = (shell.clone(), pasta.clone(), close.clone());
                let b = acao(tr("See the biggest"));
                b.connect_clicked(move |_| {
                    c();
                    if let Some(s) = s.upgrade() {
                        s.abrir_arquivos(p.as_deref());
                    }
                });
                Some(b)
            }
            "modelos" => {
                let (s, c) = (shell.clone(), close.clone());
                let b = acao(tr("Manage"));
                b.connect_clicked(move |_| {
                    c();
                    if let Some(s) = s.upgrade() {
                        s.abrir_configuracoes(Some("agentes"));
                    }
                });
                Some(b)
            }
            "projetos" if r.builds_size > 0 => Some(acao(&trf("Clean builds · {size}", &[("size", size(r.builds_size))]))),
            "outros" => {
                let (s, p, c) = (shell.clone(), pasta.clone(), close.clone());
                let b = acao(tr("See"));
                b.connect_clicked(move |_| {
                    c();
                    if let Some(s) = s.upgrade() {
                        s.abrir_arquivos(p.as_deref());
                    }
                });
                Some(b)
            }
            _ => None,
        };
        if let Some(b) = b {
            if *key == "projetos" {
                wire_free(&b, r, liberar_once.clone());
            }
            crate::central::posicao_debug(&format!("cmd ui {key}"), b.upcast_ref());
            row.append(&b);
        }
        linhas.push(row.upcast());
    }
    cascade(&rows, linhas);
    card.append(&rows);
    // what can go without losing anything of the person's: the build caches
    if r.builds_size > 0 {
        let s = caixa(Orientation::Horizontal, 10, "cmd-ui-sugestao");
        s.append(&ui_icon("spark", "#A5A1E2", 16));
        let col = caixa(Orientation::Vertical, 2, "");
        col.set_hexpand(true);
        let a = rotulo(&trf("I can free {size} without deleting anything of yours", &[("size", size(r.builds_size))]), "cmd-ui-nome");
        a.set_xalign(0.0);
        col.append(&a);
        let n = r.builds.len();
        let d = rotulo(&if n == 1 { tr("the build cache of 1 project (it is built again when needed)").to_string() } else { trf("the build caches of {n} projects (built again when needed)", &[("n", n.to_string())]) }, "cmd-ui-sub");
        d.set_xalign(0.0);
        d.set_wrap(true);
        col.append(&d);
        s.append(&col);
        let b = gtk::Button::with_label(&trf("Free {size}", &[("size", size(r.builds_size))]));
        b.add_css_class("cmd-ui-liberar");
        b.set_valign(Align::Center);
        wire_free(&b, r, liberar_once);
        crate::central::posicao_debug("cmd ui liberar", b.upcast_ref());
        s.append(&b);
        card.append(&s);
    }
    let pe = caixa(Orientation::Horizontal, 10, "");
    // the same, kept on the desktop: its Storage widget
    pe.append(&pin_button(shell.clone(), "armazenamento", crate::widgets::Tam::M));
    let mon = gtk::Button::with_label(tr("Open in the Monitor"));
    mon.add_css_class("cmd-ui-link");
    let (sh, c) = (shell.clone(), close.clone());
    mon.connect_clicked(move |_| {
        c();
        if let Some(s) = sh.upgrade() {
            crate::monitor::open(&s, Some("discos"));
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
        eprintln!("debug comando: interface do disco ({} · {} livres · builds {})", headline(r), size(r.free), size(r.builds_size));
    }
    card.upcast()
}

/// "Liberar": a first click asks, the second deletes the build caches (only folders Cargo marked as caches).
fn wire_free(b: &gtk::Button, r: &Report, asked: Rc<Cell<bool>>) {
    let builds = r.builds.clone();
    let total = r.builds_size;
    b.connect_clicked(move |b| {
        if !asked.replace(true) {
            b.set_label(&trf("Confirm: delete {size} of builds", &[("size", size(total))]));
            return;
        }
        let builds = builds.clone();
        b.set_sensitive(false);
        let bw = b.downgrade();
        crate::lateral::em_segundo_plano(move || {
            let mut feitos = 0;
            for t in &builds {
                // (a folder Cargo marked as its cache, and nothing else)
                if t.join("CACHEDIR.TAG").is_file() && t.file_name().is_some_and(|n| n == "target") && std::fs::remove_dir_all(t).is_ok() {
                    feitos += 1;
                }
            }
            feitos
        }, move |feitos| {
            if let Some(b) = bw.upgrade() {
                b.set_label(&trf("Freed {size}", &[("size", size(total))]));
            }
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug comando: builds apagados ({feitos})");
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_question() {
        assert!(is_question("o que está ocupando meu disco?"));
        assert!(is_question("espaço em disco"));
        assert!(is_question("what is using my disk space"));
        assert!(!is_question("abrir o disco de backup"));
        assert!(!is_question("quanto custa um ssd"));
    }

    #[test]
    fn headline_names_the_biggest() {
        let r = Report { total: 1000, free: 280, parts: vec![
            ("videos", "Vídeos".into(), 400, None), ("modelos", "Modelos de IA".into(), 320, None), ("projetos", "Projetos".into(), 0, None), ("outros", "Outros".into(), 0, None),
        ], ..Default::default() };
        let h = headline(&r);
        assert!(h.contains("Vídeos") && h.contains("72"), "{h}");
    }
}
