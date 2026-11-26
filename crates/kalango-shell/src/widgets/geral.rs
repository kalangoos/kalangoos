//! System widgets (design "Widgets › Sistema"): today's agenda, what is playing, the weather, the focus timer, the
//! batteries of the devices, a quick note, quick actions and the month's calendar. Real sources only: the local
//! calendars, MPRIS, Open-Meteo for the city the person picked, the focus timer the "Hoje" bar shares, UPower, a note
//! file of the person's.

use super::comum::{barra, cabecalho, Anel, Suave, Texto, ACCENT, CORAL, GOLD, MUTED, TEAL};
use super::{quadro, Tam};
use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

/// Runs `f` now and every `s` seconds while `w` exists.
fn a_cada(w: &impl IsA<gtk::Widget>, s: u32, f: impl Fn() + 'static) {
    f();
    let w = w.as_ref().downgrade();
    glib::timeout_add_seconds_local(s, move || {
        if w.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        f();
        glib::ControlFlow::Continue
    });
}

/// One event row: colour bar, title, time.
fn evento(i: usize, e: &crate::agenda::Event) -> gtk::Box {
    let row = caixa(Orientation::Horizontal, 10, "");
    let cor = caixa(Orientation::Vertical, 0, "desk-evento-cor");
    cor.add_css_class(["acento", "verde", "ouro"][i % 3]);
    row.append(&cor);
    let col = caixa(Orientation::Vertical, 1, "");
    col.set_hexpand(true);
    let t = rotulo(&e.title, "desk-metrica-nome");
    t.set_xalign(0.0);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    t.set_max_width_chars(1);
    t.set_hexpand(true);
    col.append(&t);
    let h = rotulo(&e.start.map(|m| format!("{:02}:{:02}", m / 60, m % 60)).unwrap_or_else(|| tr("all day").to_string()), "desk-metrica-sub");
    h.set_xalign(0.0);
    col.append(&h);
    row.append(&col);
    row
}

pub fn hoje() -> gtk::Widget {
    let (w, h) = Tam::M.px();
    let q = quadro(w, h);
    let (cab, titulo) = cabecalho("relogio", "");
    q.append(&cab);
    let lista = caixa(Orientation::Vertical, 10, "");
    q.append(&lista);
    let ultimo: RefCell<Vec<crate::agenda::Event>> = RefCell::new(vec![crate::agenda::Event { start: Some(u32::MAX), ..Default::default() }]);
    let (t, l) = (titulo.clone(), lista.clone());
    a_cada(&q, 60, move || {
        let dia = glib::DateTime::now_local().ok().and_then(|d| d.format("%A %-d").ok()).map(|s| s.to_uppercase()).unwrap_or_default();
        t.set_text(&format!("{} · {dia}", tr("TODAY")));
        let ev = crate::agenda::today();
        if *ultimo.borrow() == ev {
            return;
        }
        while let Some(c) = l.first_child() {
            l.remove(&c);
        }
        if ev.is_empty() {
            let v = rotulo(tr("Nothing on the calendar today."), "desk-metrica-sub");
            v.set_xalign(0.0);
            l.append(&v);
        }
        for (i, e) in ev.iter().take(3).enumerate() {
            l.append(&evento(i, e));
        }
        *ultimo.borrow_mut() = ev;
    });
    q.upcast()
}

pub fn tocando(shell: &Rc<Shell>) -> gtk::Widget {
    let (w, h) = Tam::M.px();
    let q = quadro(w, h);
    q.append(&cabecalho("musica", tr("NOW PLAYING")).0);
    let faixa = caixa(Orientation::Horizontal, 12, "");
    // (the cover slides when the track changes, as in the Music app and the island)
    let capa = crate::midia::CapaDeslizante::new(56);
    capa.widget().add_css_class("desk-capa");
    faixa.append(capa.widget());
    let textos = caixa(Orientation::Vertical, 2, "");
    textos.set_valign(Align::Center);
    textos.set_hexpand(true);
    let titulo = rotulo("", "desk-midia-titulo");
    let artista = rotulo("", "desk-midia-artista");
    for l in [&titulo, &artista] {
        l.set_xalign(0.0);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l.set_max_width_chars(1);
        l.set_hexpand(true);
        textos.append(l);
    }
    faixa.append(&textos);
    q.append(&faixa);
    let (b, progresso) = barra(344, CORAL);
    q.append(&b);
    let controles = caixa(Orientation::Horizontal, 18, "");
    controles.set_halign(Align::Center);
    let tocar = gtk::Image::new();
    for (glifo, cmd) in [("anterior", "Previous"), ("pausa", "PlayPause"), ("proxima", "Next")] {
        let bt = gtk::Button::new();
        bt.add_css_class("desk-midia-botao");
        let i = ui_icon(glifo, "#E2E2E6", 16);
        if cmd == "PlayPause" {
            tocar.set_paintable(i.paintable().as_ref());
            bt.set_child(Some(&tocar));
        } else {
            bt.set_child(Some(&i));
        }
        let s = Rc::downgrade(shell);
        bt.connect_clicked(move |_| {
            if let Some(s) = s.upgrade() {
                s.midia.comando(cmd);
            }
        });
        controles.append(&bt);
    }
    q.append(&controles);
    let nada = rotulo(tr("Nothing playing"), "desk-metrica-sub");
    nada.set_xalign(0.0);
    q.append(&nada);
    let mostrar = {
        let (q, capa, titulo, artista, b, controles, nada, tocar) = (q.downgrade(), capa, titulo, artista, b.clone(), controles, nada, tocar);
        move |m: Option<crate::midia::Midia>| {
            if q.upgrade().is_none() {
                return;
            }
            let tem = m.is_some();
            for w in [faixa.upcast_ref::<gtk::Widget>(), b.upcast_ref(), controles.upcast_ref()] {
                w.set_visible(tem);
            }
            nada.set_visible(!tem);
            let Some(m) = m else { return };
            titulo.set_text(&m.titulo);
            artista.set_text(&m.artista);
            capa.mostrar(&m);
            tocar.set_paintable(ui_icon(if m.tocando { "pausa" } else { "tocar" }, "#E2E2E6", 16).paintable().as_ref());
        }
    };
    shell.midia.ouvir(mostrar);
    // the position, every second (players do not announce it)
    let (s, b2, p) = (Rc::downgrade(shell), b.clone(), progresso.clone());
    a_cada(&q, 1, move || {
        let Some(s) = s.upgrade() else { return };
        let (b2, p) = (b2.clone(), p.clone());
        s.midia.posicao(move |r| {
            if let Some((pos, len)) = r.filter(|(_, l)| *l > 0) {
                p.ir(pos as f64 / len as f64, &b2);
            }
        });
    });
    q.upcast()
}

pub fn clima(shell: &Rc<Shell>) -> gtk::Widget {
    let (w, h) = Tam::P.px();
    let q = quadro(w, h);
    let noite = glib::DateTime::now_local().map(|d| !(6..18).contains(&d.hour())).unwrap_or(false);
    let (cab, titulo) = cabecalho(if noite { "lua-pequena" } else { "sol" }, tr("WEATHER"));
    q.append(&cab);
    let agora = caixa(Orientation::Horizontal, 8, "");
    let temp = rotulo("", "desk-clima-temp");
    agora.append(&temp);
    let ceu = rotulo("", "desk-metrica-sub");
    ceu.set_valign(Align::End);
    ceu.set_margin_bottom(8);
    ceu.set_ellipsize(gtk::pango::EllipsizeMode::End);
    ceu.set_max_width_chars(1);
    ceu.set_hexpand(true);
    ceu.set_xalign(0.0);
    agora.append(&ceu);
    q.append(&agora);
    let horas = caixa(Orientation::Horizontal, 0, "");
    horas.set_homogeneous(true);
    q.append(&horas);
    let minmax = rotulo("", "desk-metrica-sub");
    minmax.set_xalign(0.0);
    q.append(&minmax);
    let escolher = gtk::Button::with_label(tr("Pick a city in the Today bar"));
    escolher.add_css_class("desk-link");
    let s = Rc::downgrade(shell);
    escolher.connect_clicked(move |_| {
        if let Some(s) = s.upgrade() {
            s.today_bar().alternar();
        }
    });
    q.append(&escolher);
    let vivo = q.downgrade();
    a_cada(&q, 1800, move || {
        let (titulo, temp, ceu, horas, minmax, escolher, agora_b, vivo) = (titulo.clone(), temp.clone(), ceu.clone(), horas.clone(), minmax.clone(), escolher.clone(), agora.clone(), vivo.clone());
        crate::lateral::em_segundo_plano(|| crate::clima::city().map(|c| { let f = crate::clima::forecast(&c); (c, f) }), move |r| {
            if vivo.upgrade().is_none() {
                return;
            }
            let tem = matches!(r, Some((_, Ok(_))));
            for w in [agora_b.upcast_ref::<gtk::Widget>(), horas.upcast_ref(), minmax.upcast_ref()] {
                w.set_visible(tem);
            }
            escolher.set_visible(r.is_none());
            match r {
                Some((c, Ok(f))) => {
                    titulo.set_text(&format!("{} · {}", tr("WEATHER"), c.name.to_uppercase()));
                    temp.set_text(&format!("{:.0}°", f.temp));
                    ceu.set_text(tr(crate::clima::describe(f.code)));
                    while let Some(x) = horas.first_child() {
                        horas.remove(&x);
                    }
                    for (hh, t) in f.hours.iter().take(4) {
                        let col = caixa(Orientation::Vertical, 2, "");
                        let a = rotulo(&format!("{t:.0}°"), "desk-metrica-nome");
                        let b = rotulo(&format!("{hh:02}h"), "desk-clima-hora");
                        a.set_xalign(0.0);
                        b.set_xalign(0.0);
                        col.append(&a);
                        col.append(&b);
                        horas.append(&col);
                    }
                    minmax.set_text(&trf("max {max}° · min {min}°", &[("max", format!("{:.0}", f.max)), ("min", format!("{:.0}", f.min))]));
                }
                Some((c, Err(_))) => {
                    titulo.set_text(&format!("{} · {}", tr("WEATHER"), c.name.to_uppercase()));
                    agora_b.set_visible(true);
                    temp.set_text("—");
                    ceu.set_text(tr("no connection"));
                }
                None => titulo.set_text(tr("WEATHER")),
            }
        });
    });
    q.upcast()
}

pub fn foco(shell: &Rc<Shell>) -> gtk::Widget {
    let (w, h) = Tam::P.px();
    let q = quadro(w, h);
    q.append(&cabecalho("relogio", tr("FOCUS")).0);
    let anel = Anel::new(96, ACCENT);
    let tempo = Texto::new("desk-anel-numero");
    tempo.alinhar(0.5);
    anel.centro.append(&tempo.stack);
    let estado = rotulo("", "desk-metrica-sub");
    anel.centro.append(&estado);
    anel.widget.set_tooltip_text(Some(tr("Click to start or pause; double click to reset")));
    q.append(&anel.widget);
    let sub = rotulo("", "desk-metrica-sub");
    sub.set_xalign(0.0);
    q.append(&sub);
    let g = gtk::GestureClick::new();
    let s = Rc::downgrade(shell);
    g.connect_pressed(move |_, n, _, _| {
        if let Some(s) = s.upgrade() {
            if n >= 2 { s.today_bar().foco_zerar() } else { s.today_bar().foco_alternar() }
        }
    });
    anel.widget.add_controller(g);
    let s = Rc::downgrade(shell);
    a_cada(&q, 1, move || {
        let Some(s) = s.upgrade() else { return };
        let (rest, total, rodando, sessoes) = s.today_bar().foco_estado();
        anel.ir(rest as f64 / total.max(1) as f64);
        tempo.definir(&format!("{:02}:{:02}", rest / 60, rest % 60));
        estado.set_text(if rodando { tr("left") } else if rest == total { tr("ready") } else { tr("paused") });
        sub.set_text(&trf("Session {k} of 4", &[("k", (sessoes + 1).min(4).to_string())]));
    });
    q.upcast()
}

/// A battery of the person's devices (UPower): kind, name, percentage.
#[derive(Clone, Debug, PartialEq)]
pub struct Bateria {
    pub tipo: String,
    pub nome: String,
    pub pct: f64,
}

/// `upower -d`, the devices that are not the computer's own battery or power line.
pub fn baterias(saida: &str) -> Vec<Bateria> {
    let mut v = Vec::new();
    for bloco in saida.split("Device: ").skip(1) {
        let linhas: Vec<&str> = bloco.lines().map(str::trim).collect();
        let tipo = linhas.iter().find(|l| matches!(**l, "mouse" | "keyboard" | "headset" | "headphones" | "phone" | "gaming-input" | "pen" | "touchpad" | "speakers" | "tablet")).map(|s| s.to_string());
        let Some(tipo) = tipo else { continue };
        let val = |k: &str| linhas.iter().find_map(|l| l.strip_prefix(k).map(|r| r.trim().to_string()));
        if val("present:").is_some_and(|p| p == "no") {
            continue;
        }
        let Some(pct) = val("percentage:").and_then(|p| p.trim_end_matches('%').trim().replace(',', ".").parse::<f64>().ok()) else { continue };
        let nome = val("model:").filter(|m| !m.is_empty()).unwrap_or_else(|| tipo.clone());
        v.push(Bateria { tipo, nome, pct });
    }
    v
}

pub fn dispositivos() -> gtk::Widget {
    let (w, h) = Tam::M.px();
    let q = quadro(w, h);
    q.append(&cabecalho("bluetooth", tr("DEVICES")).0);
    let lista = caixa(Orientation::Vertical, 10, "");
    q.append(&lista);
    let ultimo: Rc<RefCell<Option<Vec<Bateria>>>> = Rc::default();
    let (l, vivo) = (lista.clone(), q.downgrade());
    a_cada(&q, 60, move || {
        let (l, ultimo, vivo) = (l.clone(), ultimo.clone(), vivo.clone());
        crate::lateral::em_segundo_plano(|| std::process::Command::new("upower").arg("-d").output().ok().map(|o| baterias(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default(), move |bs| {
            if vivo.upgrade().is_none() || ultimo.borrow().as_ref() == Some(&bs) {
                return;
            }
            while let Some(c) = l.first_child() {
                l.remove(&c);
            }
            if bs.is_empty() {
                let v = rotulo(tr("No device with a battery"), "desk-metrica-sub");
                v.set_xalign(0.0);
                l.append(&v);
            }
            for b in bs.iter().take(3) {
                let row = caixa(Orientation::Horizontal, 10, "");
                let g = match b.tipo.as_str() {
                    "mouse" | "touchpad" => "mouse",
                    "keyboard" => "teclado",
                    "phone" | "tablet" => "celular",
                    _ => "fone",
                };
                let i = ui_icon(g, MUTED, 16);
                i.set_valign(Align::Start);
                row.append(&i);
                let col = caixa(Orientation::Vertical, 4, "");
                col.set_hexpand(true);
                let linha = caixa(Orientation::Horizontal, 0, "");
                let n = rotulo(&b.nome, "desk-metrica-nome");
                n.set_xalign(0.0);
                n.set_hexpand(true);
                n.set_ellipsize(gtk::pango::EllipsizeMode::End);
                n.set_max_width_chars(1);
                linha.append(&n);
                linha.append(&rotulo(&format!("{:.0}%", b.pct), "desk-metrica-sub"));
                col.append(&linha);
                let (bar, v): (gtk::DrawingArea, Suave) = barra(318, if b.pct >= 50.0 { TEAL } else if b.pct >= 20.0 { GOLD } else { CORAL });
                col.append(&bar);
                v.ir(b.pct / 100.0, &bar);
                row.append(&col);
                l.append(&row);
            }
            *ultimo.borrow_mut() = Some(bs);
        });
    });
    q.upcast()
}

fn nota_path() -> PathBuf {
    glib::user_data_dir().join("kalango/nota-rapida.txt")
}

pub fn nota(shell: &Rc<Shell>) -> gtk::Widget {
    let (w, h) = Tam::M.px();
    let q = quadro(w, h);
    q.append(&cabecalho("doc", tr("QUICK NOTE")).0);
    let texto = gtk::TextView::new();
    texto.add_css_class("desk-nota");
    texto.set_wrap_mode(gtk::WrapMode::WordChar);
    texto.set_vexpand(true);
    texto.buffer().set_text(&std::fs::read_to_string(nota_path()).unwrap_or_default());
    let rolagem = gtk::ScrolledWindow::new();
    rolagem.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    rolagem.set_child(Some(&texto));
    rolagem.set_vexpand(true);
    q.append(&rolagem);
    // saved a moment after the last key
    let pendente: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    texto.buffer().connect_changed(move |b| {
        if let Some(s) = pendente.borrow_mut().take() {
            s.remove();
        }
        let t = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
        let p = pendente.clone();
        *pendente.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(800), move || {
            p.borrow_mut().take();
            let caminho = nota_path();
            if let Some(d) = caminho.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if let Err(e) = std::fs::write(&caminho, t) {
                eprintln!("kalango-shell: nota rápida: {e}");
            }
        }));
    });
    let acao = gtk::Button::new();
    acao.add_css_class("desk-link");
    let linha = caixa(Orientation::Horizontal, 6, "");
    linha.append(&crate::superficies::estrela(13, ACCENT));
    linha.append(&rotulo(tr("Turn into tasks"), "desk-link-texto"));
    acao.set_child(Some(&linha));
    acao.set_halign(Align::Start);
    let (s, t) = (Rc::downgrade(shell), texto.clone());
    acao.connect_clicked(move |_| {
        let b = t.buffer();
        let nota = b.text(&b.start_iter(), &b.end_iter(), false).trim().to_string();
        if nota.is_empty() {
            return;
        }
        if let Some(s) = s.upgrade() {
            s.right_bar().ask_agent(&format!("{}\n\n{nota}", tr("Turn these notes into a list of tasks, one per line, each starting with a verb:")));
        }
    });
    q.append(&acao);
    q.upcast()
}

pub fn acoes(shell: &Rc<Shell>) -> gtk::Widget {
    let (w, h) = Tam::M.px();
    let q = quadro(w, h);
    q.append(&cabecalho("raio", tr("QUICK ACTIONS")).0);
    let grade = gtk::Grid::new();
    grade.set_row_spacing(8);
    grade.set_column_spacing(8);
    grade.set_column_homogeneous(true);
    let botao = |glifo: &str, texto: &str, principal: bool| {
        let b = gtk::Button::new();
        b.add_css_class("desk-acao");
        if principal {
            b.add_css_class("principal");
        }
        let l = caixa(Orientation::Horizontal, 8, "");
        if glifo == "spark" {
            l.append(&crate::superficies::estrela(13, ACCENT));
        } else {
            l.append(&ui_icon(glifo, MUTED, 16));
        }
        let r = rotulo(texto, "desk-metrica-nome");
        l.append(&r);
        b.set_child(Some(&l));
        b.set_size_request(168, 44);
        (b, r)
    };
    let (captura, _) = botao("camera", tr("Take a screenshot"), false);
    captura.connect_clicked(|_| {
        // niri's own screenshot (pick an area, a window or the screen)
        let _ = crate::servicos::niri_acao(serde_json::json!({"Screenshot": {"show_pointer": false}}));
    });
    let (foco, foco_r) = botao("relogio", tr("Start focus"), false);
    let s = Rc::downgrade(shell);
    foco.connect_clicked(move |_| {
        if let Some(s) = s.upgrade() {
            s.today_bar().foco_alternar();
        }
    });
    let (nova, _) = botao("doc", tr("New note"), false);
    nova.connect_clicked(|_| {
        let p = nota_path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if !p.exists() {
            let _ = std::fs::write(&p, "");
        }
        let _ = crate::apps::open_default(&gtk::gio::File::for_path(&p), gtk::gdk::Display::default().map(|d| d.app_launch_context()).as_ref().map(|c| c.upcast_ref()));
    });
    let (resumir, _) = botao("spark", tr("Summarize this screen"), true);
    let s = Rc::downgrade(shell);
    resumir.connect_clicked(move |_| {
        if let Some(s) = s.upgrade() {
            super::visao_da_tela(&s);
        }
    });
    grade.attach(&captura, 0, 0, 1, 1);
    grade.attach(&foco, 1, 0, 1, 1);
    grade.attach(&nova, 0, 1, 1, 1);
    grade.attach(&resumir, 1, 1, 1, 1);
    q.append(&grade);
    // "Iniciar foco" says what a click would do now
    let s = Rc::downgrade(shell);
    a_cada(&q, 1, move || {
        if let Some(s) = s.upgrade() {
            foco_r.set_text(if s.today_bar().foco_estado().2 { tr("Pause focus") } else { tr("Start focus") });
        }
    });
    q.upcast()
}

pub fn calendario() -> gtk::Widget {
    let (w, h) = Tam::G.px();
    let q = quadro(w, h);
    let topo = caixa(Orientation::Horizontal, 0, "");
    let (cab, titulo) = cabecalho("relogio", "");
    cab.set_hexpand(true);
    topo.append(&cab);
    let hoje_b = gtk::Button::with_label(tr("today"));
    hoje_b.add_css_class("desk-link");
    topo.append(&hoje_b);
    q.append(&topo);
    let grade = gtk::Grid::new();
    grade.set_column_homogeneous(true);
    grade.set_row_spacing(2);
    q.append(&grade);
    let eventos = caixa(Orientation::Vertical, 10, "");
    q.append(&eventos);
    // which month is shown (scrolling on the grid moves it)
    let mes: Rc<Cell<(i32, i32)>> = Rc::new(Cell::new((0, 0)));
    let desenhar: Rc<dyn Fn()> = {
        let (mes, titulo, grade, eventos, hoje_b) = (mes.clone(), titulo.clone(), grade.clone(), eventos.clone(), hoje_b.clone());
        Rc::new(move || {
            let Ok(agora) = glib::DateTime::now_local() else { return };
            let (mut y, mut m) = mes.get();
            if y == 0 {
                y = agora.year();
                m = agora.month();
                mes.set((y, m));
            }
            hoje_b.set_visible((y, m) != (agora.year(), agora.month()));
            let Ok(primeiro) = glib::DateTime::from_local(y, m, 1, 0, 0, 0.0) else { return };
            titulo.set_text(&primeiro.format("%B %Y").map(|s| s.to_uppercase()).unwrap_or_default());
            while let Some(c) = grade.first_child() {
                grade.remove(&c);
            }
            // week starts on Sunday (as in the design)
            for (i, d) in [tr("S_sunday"), tr("M_monday"), tr("T_tuesday"), tr("W_wednesday"), tr("T_thursday"), tr("F_friday"), tr("S_saturday")].iter().enumerate() {
                grade.attach(&rotulo(d, "desk-cal-semana"), i as i32, 0, 1, 1);
            }
            let comeco = primeiro.day_of_week() % 7; // Sunday = 0
            let inicio = primeiro.add_days(-comeco).unwrap_or(primeiro.clone());
            let com_eventos = crate::agenda::days_with_events(y, m);
            for k in 0..42 {
                let Ok(d) = inicio.add_days(k) else { continue };
                let c = caixa(Orientation::Vertical, 0, "desk-cal-dia");
                let n = rotulo(&d.day_of_month().to_string(), "desk-cal-numero");
                if d.month() != m {
                    n.add_css_class("fora");
                }
                if (d.year(), d.month(), d.day_of_month()) == (agora.year(), agora.month(), agora.day_of_month()) {
                    n.add_css_class("hoje");
                }
                n.set_halign(Align::Center);
                c.append(&n);
                let ponto = caixa(Orientation::Vertical, 0, "desk-cal-ponto");
                ponto.set_halign(Align::Center);
                ponto.set_opacity(if d.month() == m && com_eventos.contains(&d.day_of_month()) { 1.0 } else { 0.0 });
                c.append(&ponto);
                grade.attach(&c, k % 7, 1 + k / 7, 1, 1);
            }
            while let Some(c) = eventos.first_child() {
                eventos.remove(&c);
            }
            for (i, e) in crate::agenda::today().iter().take(2).enumerate() {
                eventos.append(&evento(i, e));
            }
        })
    };
    let d = desenhar.clone();
    let (mes2, d2) = (mes.clone(), desenhar.clone());
    hoje_b.connect_clicked(move |_| {
        mes2.set((0, 0));
        d2();
    });
    let rolar = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE);
    let (mes3, d3) = (mes.clone(), desenhar.clone());
    rolar.connect_scroll(move |_, _, dy| {
        let (y, m) = mes3.get();
        let t = y * 12 + (m - 1) + if dy > 0.0 { 1 } else { -1 };
        mes3.set((t.div_euclid(12), t.rem_euclid(12) + 1));
        d3();
        glib::Propagation::Stop
    });
    grade.add_controller(rolar);
    a_cada(&q, 600, move || d());
    let _: Weak<()> = Weak::new();
    q.upcast()
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_upower_devices() {
        let saida = "Device: /org/freedesktop/UPower/devices/battery_BAT0\n  native-path: BAT0\n  power supply: yes\n  battery\n    percentage: 80%\n\n\
Device: /org/freedesktop/UPower/devices/mouse_dev_1\n  model: MX Master 3\n  power supply: no\n  mouse\n    present: yes\n    percentage: 45%\n\n\
Device: /org/freedesktop/UPower/devices/headset_dev_2\n  model: WH-1000XM4\n  headset\n    percentage: 80,5%\n";
        let b = super::baterias(saida);
        assert_eq!(b.len(), 2);
        assert_eq!((b[0].tipo.as_str(), b[0].nome.as_str(), b[0].pct), ("mouse", "MX Master 3", 45.0));
        assert_eq!((b[1].tipo.as_str(), b[1].pct), ("headset", 80.5));
    }
}
