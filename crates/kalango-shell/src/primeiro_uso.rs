//! KalangoOS · first use (designs "Primeiro uso · 1 Olá", "2 Aparência", "3 Agentes"): the first session after the
//! installation, a card like the installer's. Language, keyboard and account came from the installation; here the
//! person brings their things from the system that was on the disk (folders and the browser, through kalangod: the
//! other disk is only read), chooses how it looks (theme, accent, dock, widgets — it changes at once) and how the
//! agents work (where they think, how much they decide alone: the permissions' columns).
//!
//! Shown when the installer left ~/.config/kalango/primeiro-uso.json with "feito": false (or by
//! `kalango-shell primeiro-uso`); "Começar" marks it done. Debug: "debug primeiro uso: …" and the controls' places.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use crate::tema::{self, DESTAQUES};
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use kalango_i18n::{n_, tr, trf};
use kalango_movimento::{Animavel, Canal, LENTA, SUAVE};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

extern "C" {
    fn gdk_wayland_toplevel_set_application_id(toplevel: *mut gdk::ffi::GdkToplevel, id: *const std::ffi::c_char);
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

fn arquivo() -> std::path::PathBuf {
    glib::user_config_dir().join("kalango/primeiro-uso.json")
}

/// What the installer left for the first use (and whether it is still to be done).
pub fn pendente() -> Option<Value> {
    let v: Value = std::fs::read(arquivo()).ok().and_then(|b| serde_json::from_slice(&b).ok())?;
    (v["feito"] == false).then_some(v)
}

fn marcar_feito() {
    let mut v: Value = std::fs::read(arquivo()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({}));
    v["feito"] = json!(true);
    if let Some(d) = arquivo().parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(arquivo(), serde_json::to_string_pretty(&v).unwrap_or_default());
}

const PASSOS: &[&str] = &[n_("Hello"), n_("Appearance"), n_("Agents")];

/// Where the agents think: (id, glyph, name, what it means).
const MODOS: &[(&str, &str, &str, &str)] = &[
    ("local", "cpu", n_("Local"), n_("private: nothing leaves the computer")),
    ("hibrido", "spark", n_("Hybrid mode"), n_("local by default; cloud only when you allow")),
    ("nuvem", "agrupar", n_("Cloud"), n_("more capable; sends what you ask")),
];

/// How much they decide alone: (id, name, what it means, the permissions' columns).
const AUTONOMIA: &[(&str, &str, &str, &[(&str, &str)])] = &[
    ("sempre", n_("Always ask"), n_("nothing happens without your OK"),
        &[("ler", "perguntar"), ("escrever", "perguntar"), ("apagar", "perguntar"), ("rede", "perguntar"), ("terminal", "perguntar"), ("tela", "perguntar"), ("gastar", "perguntar")]),
    ("risco", n_("Ask only for risky actions"), n_("delete, send, install, spend"),
        &[("ler", "livre"), ("escrever", "livre"), ("tela", "livre"), ("apagar", "perguntar"), ("rede", "perguntar"), ("terminal", "perguntar"), ("gastar", "perguntar")]),
    ("autonomo", n_("Autonomous within the permissions"), n_("you review it later in the Timeline"),
        &[("ler", "livre"), ("escrever", "livre"), ("tela", "livre"), ("apagar", "livre"), ("rede", "livre"), ("terminal", "livre"), ("gastar", "perguntar")]),
];

thread_local! {
    static ABERTO: RefCell<Option<Rc<PrimeiroUso>>> = const { RefCell::new(None) };
}

/// Opens the first use (or brings it forward).
pub fn abrir(app: &gtk::Application) {
    if let Some(p) = ABERTO.with(|a| a.borrow().clone()) {
        p.janela.present();
        return;
    }
    let p = PrimeiroUso::new(app);
    // closed before the end (the compositor's close): it opens again at the next session, until "Começar"
    p.janela.connect_close_request(|_| {
        ABERTO.with(|a| a.borrow_mut().take());
        LUGARES.with(|l| l.borrow_mut().clear());
        glib::Propagation::Proceed
    });
    p.janela.present();
    ABERTO.with(|a| *a.borrow_mut() = Some(p));
}

pub struct PrimeiroUso {
    janela: gtk::ApplicationWindow,
    passo: Cell<usize>,
    paginas: Vec<Animavel>,
    marcas: Vec<(gtk::Box, gtk::Label)>,
    titulo: gtk::Label,
    sub: gtk::Label,
    voltar: gtk::Button,
    contagem: gtk::Label,
    seguir: gtk::Button,
    // 1
    sistema: RefCell<Option<(String, String, String)>>,
    arquivos_linha: gtk::Box,
    arquivos: gtk::Switch,
    arquivos_sub: gtk::Label,
    navegador_linha: gtk::Box,
    navegador: gtk::Switch,
    navegador_titulo: gtk::Label,
    navegador_sub: gtk::Label,
    procurando: gtk::Label,
    revisar: gtk::CheckButton,
    importando: Cell<bool>,
    // 2
    estilos: Vec<(&'static str, gtk::CheckButton, gtk::DrawingArea)>,
    destaques: Vec<(&'static str, gtk::DrawingArea)>,
    dock: Vec<(&'static str, gtk::ToggleButton)>,
    transparentes: gtk::Switch,
    lendo: Cell<bool>,
    // 3
    modos: Vec<(&'static str, gtk::ToggleButton)>,
    autonomia: Vec<(&'static str, gtk::CheckButton)>,
    nuvem_nota: gtk::Label,
}

thread_local! {
    /// the controls whose places the session test reads (logged again when the card's content moves)
    static LUGARES: RefCell<Vec<(String, glib::WeakRef<gtk::Widget>)>> = const { RefCell::new(Vec::new()) };
}

/// A control's place in the window, for the session test.
fn lugar(nome: &str, w: &impl IsA<gtk::Widget>) {
    if debug() {
        LUGARES.with(|l| l.borrow_mut().push((nome.to_string(), w.upcast_ref::<gtk::Widget>().downgrade())));
        crate::central::posicao_debug(&format!("pu {nome}"), w.upcast_ref());
    }
}

/// The places again, once the content moved (a row that appeared, another step).
fn relogar_lugares() {
    if !debug() {
        return;
    }
    LUGARES.with(|l| {
        for (n, w) in l.borrow().iter() {
            if let Some(w) = w.upgrade().filter(|w| w.is_mapped()) {
                crate::central::posicao_debug(&format!("pu {n}"), &w);
            }
        }
    });
}

fn linha(icone: &str, titulo: &gtk::Label, sub: &gtk::Label, controle: &impl IsA<gtk::Widget>) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 12, "inst-linha");
    let i = ui_icon(icone, "#B9BAC4", 16);
    i.set_valign(Align::Center);
    r.append(&i);
    let col = caixa(Orientation::Vertical, 1, "");
    col.set_hexpand(true);
    col.set_valign(Align::Center);
    titulo.add_css_class("inst-texto");
    titulo.set_xalign(0.0);
    col.append(titulo);
    sub.add_css_class("inst-sub");
    sub.set_xalign(0.0);
    sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
    col.append(sub);
    r.append(&col);
    controle.set_valign(Align::Center);
    r.append(controle);
    r
}

fn interruptor() -> gtk::Switch {
    let s = gtk::Switch::new();
    s.add_css_class("inst-switch");
    s
}

/// The scene behind a theme card: the dark or light picture, cut to the card (Automático: half of each).
fn desenhar_cena(cr: &gtk::cairo::Context, w: f64, h: f64, estilo: &str, cenas: &[Option<gtk::gdk_pixbuf::Pixbuf>; 2]) {
    let pinta = |cr: &gtk::cairo::Context, pb: &Option<gtk::gdk_pixbuf::Pixbuf>, x0: f64, largura: f64, cor: (f64, f64, f64)| {
        cr.save().ok();
        cr.rectangle(x0, 0.0, largura, h);
        cr.clip();
        match pb {
            Some(pb) => {
                let k = (w / pb.width() as f64).max(h / pb.height() as f64);
                cr.scale(k, k);
                cr.set_source_pixbuf(pb, ((w / k) - pb.width() as f64) / 2.0, ((h / k) - pb.height() as f64) / 2.0);
            }
            None => cr.set_source_rgb(cor.0, cor.1, cor.2),
        }
        let _ = cr.paint();
        cr.restore().ok();
    };
    let r = 4.0;
    cr.new_sub_path();
    cr.arc(w - r, r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(w - r, h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(r, h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(r, r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
    cr.close_path();
    cr.clip();
    match estilo {
        "claro" => pinta(cr, &cenas[1], 0.0, w, (0.93, 0.94, 0.96)),
        "automatico" => {
            pinta(cr, &cenas[0], 0.0, (w / 2.0).round(), (0.09, 0.09, 0.11));
            pinta(cr, &cenas[1], (w / 2.0).round(), w - (w / 2.0).round(), (0.93, 0.94, 0.96));
        }
        _ => pinta(cr, &cenas[0], 0.0, w, (0.09, 0.09, 0.11)),
    }
}

impl PrimeiroUso {
    fn new(app: &gtk::Application) -> Rc<PrimeiroUso> {
        let janela = gtk::ApplicationWindow::new(app);
        janela.set_title(Some(tr("Welcome to KalangoOS")));
        janela.set_default_size(1040, 660);
        janela.set_resizable(false);
        janela.add_css_class("boas-vindas");
        let nada = gtk::Box::new(Orientation::Horizontal, 0);
        nada.set_visible(false);
        janela.set_titlebar(Some(&nada));
        let app_id = |j: &gtk::ApplicationWindow| {
            if let Some(s) = j.surface().filter(|s| s.type_().name().contains("Wayland")) {
                let id = std::ffi::CString::new("os.kalango.BoasVindas").unwrap();
                unsafe { gdk_wayland_toplevel_set_application_id(s.as_ptr() as *mut gdk::ffi::GdkToplevel, id.as_ptr()) };
            }
        };
        janela.connect_realize(move |j| app_id(j));
        janela.connect_map(move |j| app_id(j));
        let raiz = caixa(Orientation::Horizontal, 0, "");
        // the left: the mark, the version, what it is (the installer's)
        let esq = caixa(Orientation::Vertical, 14, "inst-lateral");
        esq.set_size_request(340, -1);
        let topo = caixa(Orientation::Vertical, 0, "");
        topo.set_vexpand(true);
        esq.append(&topo);
        let logo = gtk::Image::from_paintable(gdk::Texture::from_bytes(&glib::Bytes::from_static(
            // the brand's light-theme logo in the light theme
            if crate::tema::e_claro() { include_bytes!("../assets/logo-claro.png") } else { include_bytes!("../assets/logo.png") })).ok().as_ref());
        logo.set_pixel_size(264);
        logo.set_halign(Align::Center);
        esq.append(&logo);
        let versao = rotulo(&trf("Version {v}", &[("v", env!("CARGO_PKG_VERSION").trim_end_matches(".0").to_string())]), "inst-versao");
        versao.set_margin_top(18);
        esq.append(&versao);
        let t = rotulo(tr("Light, fast and made to work with agents."), "inst-sub");
        t.set_justify(gtk::Justification::Center);
        t.set_wrap(true);
        t.set_max_width_chars(26);
        esq.append(&t);
        let base = caixa(Orientation::Vertical, 0, "");
        base.set_vexpand(true);
        esq.append(&base);
        let alca = gtk::WindowHandle::new();
        alca.set_child(Some(&esq));
        raiz.append(&alca);
        let dir = caixa(Orientation::Vertical, 0, "inst-direita");
        dir.set_hexpand(true);
        let stepper = caixa(Orientation::Horizontal, 8, "inst-passos");
        let mut marcas = Vec::new();
        for (i, p) in PASSOS.iter().enumerate() {
            if i > 0 {
                let l = caixa(Orientation::Horizontal, 0, "inst-passo-linha");
                l.set_size_request(24, 1);
                l.set_valign(Align::Center);
                stepper.append(&l);
            }
            let m = caixa(Orientation::Horizontal, 6, "inst-passo");
            let n = caixa(Orientation::Horizontal, 0, "inst-passo-num");
            n.set_size_request(18, 18);
            n.set_valign(Align::Center);
            n.set_homogeneous(true);
            n.append(&gtk::Label::new(Some(&(i + 1).to_string())));
            m.append(&n);
            let txt = rotulo(tr(p), "");
            m.append(&txt);
            stepper.append(&m);
            marcas.push((n, txt));
        }
        let sh = gtk::WindowHandle::new();
        sh.set_child(Some(&stepper));
        dir.append(&sh);
        let titulo = rotulo("", "inst-titulo");
        titulo.set_xalign(0.0);
        titulo.set_margin_top(18);
        dir.append(&titulo);
        let sub = rotulo("", "inst-sub");
        sub.set_xalign(0.0);
        sub.set_wrap(true);
        sub.set_max_width_chars(70);
        sub.set_margin_top(4);
        dir.append(&sub);
        let palco = caixa(Orientation::Vertical, 0, "");
        palco.set_vexpand(true);
        palco.set_margin_top(18);
        palco.set_overflow(gtk::Overflow::Hidden);
        dir.append(&palco);
        let rodape = caixa(Orientation::Horizontal, 12, "inst-rodape");
        let voltar = gtk::Button::new();
        voltar.add_css_class("inst-voltar");
        let vc = caixa(Orientation::Horizontal, 6, "");
        vc.append(&ui_icon("seta-esquerda", "#B9BAC4", 14));
        vc.append(&rotulo(tr("Back"), ""));
        voltar.set_child(Some(&vc));
        rodape.append(&voltar);
        let esp = caixa(Orientation::Horizontal, 0, "");
        esp.set_hexpand(true);
        rodape.append(&esp);
        let contagem = rotulo("", "inst-contagem");
        contagem.set_valign(Align::Center);
        rodape.append(&contagem);
        let seguir = gtk::Button::with_label(tr("Continue"));
        seguir.add_css_class("inst-seguir");
        rodape.append(&seguir);
        dir.append(&rodape);
        raiz.append(&dir);
        janela.set_child(Some(&raiz));

        // ---- 1 · hello: what to bring
        let p1 = caixa(Orientation::Vertical, 14, "");
        let cartao = caixa(Orientation::Vertical, 2, "inst-cartao");
        let (at, asub) = (gtk::Label::new(Some(tr("Files and folders"))), gtk::Label::new(None));
        let arquivos = interruptor();
        arquivos.set_active(true);
        let arquivos_linha = linha("pasta", &at, &asub, &arquivos);
        cartao.append(&arquivos_linha);
        let (nt, nsub) = (gtk::Label::new(None), gtk::Label::new(None));
        let navegador = interruptor();
        navegador.set_active(true);
        let navegador_linha = linha("globo", &nt, &nsub, &navegador);
        cartao.append(&navegador_linha);
        // the agents' connectors (GitHub, Git, databases…): where the person's other accounts come in
        let conectores = gtk::Button::new();
        conectores.add_css_class("inst-linha-botao");
        let (ct, csub) = (gtk::Label::new(Some(tr("Agents' connectors"))), gtk::Label::new(Some(tr("GitHub, Git, databases and more, through the agents"))));
        conectores.set_child(Some(&linha("usuario", &ct, &csub, &ui_icon("chevron", "#8E8F99", 14))));
        cartao.append(&conectores);
        p1.append(&cartao);
        let procurando = rotulo(tr("Looking for another system on the disks…"), "inst-sub");
        procurando.set_xalign(0.0);
        p1.append(&procurando);
        let revisar = gtk::CheckButton::with_label(tr("Review with the agent what was imported"));
        revisar.add_css_class("inst-radio");
        revisar.set_active(true);
        lugar("revisar", &revisar);
        p1.append(&revisar);

        // ---- 2 · appearance
        let p2 = caixa(Orientation::Vertical, 14, "");
        let cenas = Rc::new([crate::papel::cena(false), crate::papel::cena(true)].map(|p| p.and_then(|p| gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(p, 480, -1, true).ok())));
        let temas = caixa(Orientation::Horizontal, 12, "");
        temas.set_homogeneous(true);
        let mut estilos = Vec::new();
        let mut grupo: Option<gtk::CheckButton> = None;
        for (id, nome) in [("escuro", tr("Dark")), ("claro", tr("Light")), ("automatico", tr("Automatic"))] {
            let col = caixa(Orientation::Vertical, 10, "");
            let da = gtk::DrawingArea::new();
            da.set_content_height(96);
            da.add_css_class("pu-tema");
            let c2 = cenas.clone();
            da.set_draw_func(move |_, cr, w, h| desenhar_cena(cr, w as f64, h as f64, id, &c2));
            col.append(&da);
            let rb = gtk::CheckButton::with_label(nome);
            rb.add_css_class("inst-radio");
            if let Some(g) = &grupo {
                rb.set_group(Some(g));
            } else {
                grupo = Some(rb.clone());
            }
            col.append(&rb);
            let clique = gtk::GestureClick::new();
            let r2 = rb.clone();
            clique.connect_released(move |_, _, _, _| r2.set_active(true));
            da.add_controller(clique);
            lugar(&format!("tema {id}"), &da);
            temas.append(&col);
            estilos.push((id, rb, da));
        }
        p2.append(&temas);
        let cor = caixa(Orientation::Horizontal, 12, "inst-linha");
        let cl = rotulo(tr("Accent colour"), "inst-texto");
        cl.set_xalign(0.0);
        cl.set_hexpand(true);
        cor.append(&cl);
        let mut destaques = Vec::new();
        for (id, hex, _) in DESTAQUES.iter().take(6) {
            let da = gtk::DrawingArea::new();
            da.set_content_width(24);
            da.set_content_height(24);
            da.set_valign(Align::Center);
            da.set_cursor_from_name(Some("pointer"));
            let (r, g, b) = tema::rgb(hex);
            let id2 = *id;
            da.set_draw_func(move |_, cr, w, h| {
                let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
                cr.set_source_rgb(r, g, b);
                cr.arc(cx, cy, 8.0, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
                if tema::ler().destaque == id2 {
                    // the ring in the text's colour (dark on the light theme)
                    let (tr_, tg, tb) = if tema::e_claro() { (0.11, 0.114, 0.141) } else { (0.886, 0.886, 0.902) };
                    cr.set_source_rgb(tr_, tg, tb);
                    cr.set_line_width(1.5);
                    cr.arc(cx, cy, 11.0, 0.0, std::f64::consts::TAU);
                    let _ = cr.stroke();
                }
            });
            lugar(&format!("destaque {id}"), &da);
            cor.append(&da);
            destaques.push((*id, da));
        }
        p2.append(&cor);
        let dl = caixa(Orientation::Horizontal, 12, "inst-linha");
        let dt = rotulo(tr("Dock"), "inst-texto");
        dt.set_xalign(0.0);
        dt.set_hexpand(true);
        dl.append(&dt);
        let seg = caixa(Orientation::Horizontal, 0, "pu-segmento");
        seg.set_valign(Align::Center);
        let mut dock = Vec::new();
        let mut g2: Option<gtk::ToggleButton> = None;
        for (id, nome) in [("flutuante", tr("Floating")), ("fixa", tr("Fixed (taskbar)"))] {
            let b = gtk::ToggleButton::with_label(nome);
            b.add_css_class("pu-opcao");
            if let Some(g) = &g2 {
                b.set_group(Some(g));
            } else {
                g2 = Some(b.clone());
            }
            lugar(&format!("dock {id}"), &b);
            seg.append(&b);
            dock.push((id, b));
        }
        dl.append(&seg);
        p2.append(&dl);
        let wc = caixa(Orientation::Vertical, 0, "inst-cartao");
        let transparentes = interruptor();
        let (wt, ws) = (gtk::Label::new(Some(tr("Transparent widgets"))), gtk::Label::new(Some(tr("the clock and the widgets right on the wallpaper"))));
        wc.append(&linha("grade", &wt, &ws, &transparentes));
        p2.append(&wc);

        // ---- 3 · agents
        let p3 = caixa(Orientation::Vertical, 14, "");
        let mc = caixa(Orientation::Horizontal, 12, "");
        mc.set_homogeneous(true);
        let mut modos = Vec::new();
        let mut g3: Option<gtk::ToggleButton> = None;
        for (id, glifo, nome, desc) in MODOS {
            let b = gtk::ToggleButton::new();
            b.add_css_class("inst-escolha");
            let c = caixa(Orientation::Vertical, 6, "");
            let topo = caixa(Orientation::Horizontal, 8, "");
            topo.append(&ui_icon(glifo, match *id { "local" => "#8ED2C3", "hibrido" => "#A5A1E2", _ => "#E5C98C" }, 16));
            let n = rotulo(tr(nome), "inst-escolha-titulo");
            n.set_hexpand(true);
            n.set_xalign(0.0);
            topo.append(&n);
            if *id == "hibrido" {
                topo.append(&rotulo(tr("suggested"), "pu-selo"));
            }
            c.append(&topo);
            let d = rotulo(tr(desc), "inst-sub");
            d.set_xalign(0.0);
            d.set_wrap(true);
            d.set_max_width_chars(22);
            c.append(&d);
            b.set_child(Some(&c));
            if let Some(g) = &g3 {
                b.set_group(Some(g));
            } else {
                g3 = Some(b.clone());
            }
            lugar(&format!("modo {id}"), &b);
            mc.append(&b);
            modos.push((*id, b));
        }
        p3.append(&mc);
        let nuvem_nota = rotulo(tr("No cloud provider is set up yet: until then everything runs on the local model. One can be connected in Agents › Settings."), "inst-sub");
        nuvem_nota.set_xalign(0.0);
        nuvem_nota.set_wrap(true);
        nuvem_nota.set_max_width_chars(80);
        nuvem_nota.set_visible(false);
        p3.append(&nuvem_nota);
        let al = rotulo(tr("HOW MUCH THE AGENTS CAN DECIDE ALONE"), "pu-rotulo");
        al.set_xalign(0.0);
        al.set_margin_top(6);
        p3.append(&al);
        let ac = caixa(Orientation::Vertical, 2, "inst-cartao");
        let mut autonomia = Vec::new();
        let mut g4: Option<gtk::CheckButton> = None;
        for (id, nome, desc, _) in AUTONOMIA {
            let r = caixa(Orientation::Horizontal, 10, "inst-linha");
            let c = gtk::CheckButton::with_label(tr(nome));
            c.add_css_class("inst-radio");
            c.set_hexpand(true);
            if let Some(g) = &g4 {
                c.set_group(Some(g));
            } else {
                g4 = Some(c.clone());
            }
            r.append(&c);
            let d = rotulo(tr(desc), "inst-sub");
            d.set_xalign(1.0);
            r.append(&d);
            lugar(&format!("autonomia {id}"), &c);
            ac.append(&r);
            autonomia.push((*id, c));
        }
        p3.append(&ac);
        let nota = caixa(Orientation::Horizontal, 8, "pu-nota");
        nota.append(&ui_icon("escudo", "#8ED2C3", 14));
        nota.append(&rotulo(tr("Everything an agent does shows in the Timeline and can be undone."), "inst-texto"));
        p3.append(&nota);

        let paginas: Vec<Animavel> = [p1, p2, p3].into_iter().map(|p| {
            let a = Animavel::new(&p);
            a.set_vexpand(true);
            a.set_visible(false);
            palco.append(&a);
            a
        }).collect();
        let pu = Rc::new(PrimeiroUso {
            janela, passo: Cell::new(0), paginas, marcas, titulo, sub, voltar, contagem, seguir,
            sistema: RefCell::default(), arquivos_linha, arquivos, arquivos_sub: asub, navegador_linha, navegador, navegador_titulo: nt, navegador_sub: nsub, procurando, revisar,
            importando: Cell::new(false),
            estilos, destaques, dock, transparentes, lendo: Cell::new(false),
            modos, autonomia, nuvem_nota,
        });
        pu.ligar(&conectores);
        pu.preencher();
        pu.procurar();
        pu.ir(0);
        lugar("continuar", &pu.seguir);
        lugar("voltar", &pu.voltar);
        pu
    }

    fn ligar(self: &Rc<Self>, conectores: &gtk::Button) {
        let w = Rc::downgrade(self);
        self.seguir.connect_clicked(move |_| if let Some(p) = w.upgrade() { p.continuar() });
        let w = Rc::downgrade(self);
        self.voltar.connect_clicked(move |_| if let Some(p) = w.upgrade() { p.ir(p.passo.get().saturating_sub(1)) });
        conectores.connect_clicked(|_| {
            if let Some(s) = crate::shell_atual() {
                s.abrir_central(Some("ajustes"));
            }
        });
        // appearance: each choice applies at once (the whole system changes behind the card)
        for (id, rb, _) in &self.estilos {
            let (w, id) = (Rc::downgrade(self), *id);
            rb.connect_toggled(move |rb| {
                let Some(p) = w.upgrade().filter(|_| rb.is_active()) else { return };
                p.marcar_tema(id);
                if !p.lendo.get() {
                    p.tema(|t| t.estilo = id.to_string());
                }
            });
        }
        for (id, da) in &self.destaques {
            let (w, id) = (Rc::downgrade(self), *id);
            let clique = gtk::GestureClick::new();
            clique.connect_released(move |_, _, _, _| {
                if let Some(p) = w.upgrade() {
                    p.tema(|t| t.destaque = id.to_string());
                    for (_, d) in &p.destaques {
                        d.queue_draw();
                    }
                }
            });
            da.add_controller(clique);
        }
        for (id, b) in &self.dock {
            let (w, id) = (Rc::downgrade(self), *id);
            b.connect_toggled(move |b| {
                if let Some(p) = w.upgrade().filter(|p| b.is_active() && !p.lendo.get()) {
                    let mut d = crate::superficies::estilo_dock();
                    d.estilo = id.to_string();
                    crate::superficies::salvar_estilo_dock(&d);
                    crate::superficies::refazer_dock();
                    if debug() {
                        eprintln!("debug primeiro uso: dock {id}");
                    }
                    let _ = p;
                }
            });
        }
        let w = Rc::downgrade(self);
        self.transparentes.connect_state_set(move |_, on| {
            if let Some(p) = w.upgrade().filter(|p| !p.lendo.get()) {
                p.tema(|t| t.widgets_transparentes = on);
            }
            glib::Propagation::Proceed
        });
        for (id, b) in &self.modos {
            let (w, id) = (Rc::downgrade(self), *id);
            b.connect_toggled(move |b| {
                if let Some(p) = w.upgrade().filter(|_| b.is_active()) {
                    // this build has no cloud provider: said where the choice is made, never hidden
                    p.nuvem_nota.set_visible(id != "local" && !nuvem_configurada());
                }
            });
        }
    }

    fn tema(&self, f: impl FnOnce(&mut tema::Tema)) {
        let mut t = tema::ler();
        f(&mut t);
        tema::salvar(&t);
        tema::aplicar(&t);
        if debug() {
            eprintln!("debug primeiro uso: tema {} {} widgets {}", t.estilo, t.destaque, t.widgets_transparentes);
        }
    }

    /// The chosen theme's card has the accent's border.
    fn marcar_tema(&self, id: &str) {
        for (e, _, da) in &self.estilos {
            if *e == id { da.add_css_class("escolhido") } else { da.remove_css_class("escolhido") }
        }
    }

    /// The controls as things are now.
    fn preencher(&self) {
        self.lendo.set(true);
        let t = tema::ler();
        for (id, rb, _) in &self.estilos {
            rb.set_active(t.estilo == *id);
        }
        self.marcar_tema(&t.estilo);
        let d = crate::superficies::estilo_dock();
        for (id, b) in &self.dock {
            b.set_active(d.estilo == *id || (*id == "flutuante" && d.estilo != "fixa"));
        }
        self.transparentes.set_active(t.widgets_transparentes);
        let modo = crate::configuracoes::ajustes()["modo_agentes"].as_str().unwrap_or("hibrido").to_string();
        for (id, b) in &self.modos {
            b.set_active(*id == modo);
        }
        for (id, c) in &self.autonomia {
            c.set_active(*id == "risco");
        }
        self.lendo.set(false);
    }

    /// The other systems on the disks (kalangod reads them, mounted read-only).
    fn procurar(self: &Rc<Self>) {
        self.arquivos_linha.set_visible(false);
        self.navegador_linha.set_visible(false);
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(|| crate::servicos::kalangod_chamar("importar.procurar", json!({})), move |r| {
            let Some(p) = w.upgrade() else { return };
            let sistemas = r.as_ref().ok().and_then(|v| v["sistemas"].as_array().cloned()).unwrap_or_default();
            let eu = glib::user_name().to_string_lossy().to_string();
            // the system and the person there: the one with this account's name, else the first
            let escolha = sistemas.iter().flat_map(|s| s["pessoas"].as_array().cloned().unwrap_or_default().into_iter().map(move |pe| (s.clone(), pe)))
                .max_by_key(|(_, pe)| (pe["nome"].as_str() == Some(eu.as_str())) as u8);
            match escolha {
                Some((s, pe)) => {
                    let sistema = s["sistema"].as_str().unwrap_or("").to_string();
                    let usado = crate::configuracoes_sistema::human(s["usado"].as_u64().unwrap_or(0));
                    let pastas = pe["pastas"].as_array().map(|a| a.len()).unwrap_or(0);
                    p.arquivos_linha.set_visible(pastas > 0);
                    p.arquivos_sub.set_text(&trf("{size} of the {system} on this disk", &[("size", usado), ("system", sistema.clone())]));
                    let nav = pe["navegadores"].as_array().and_then(|a| a.first().cloned());
                    if let Some(n) = &nav {
                        let nome = n["nome"].as_str().unwrap_or("");
                        // Firefox's passwords come with its profile; Chromium's stay locked to the other system's keyring
                        p.navegador_titulo.set_text(if nome == "Firefox" && s["windows"] != true { tr("Browser bookmarks and passwords") } else { tr("Browser bookmarks") });
                        p.navegador_sub.set_text(&match n["favoritos"].as_u64() {
                            Some(k) => trf("{browser} · {n} bookmarks", &[("browser", nome.to_string()), ("n", k.to_string())]),
                            None => nome.to_string(),
                        });
                    }
                    p.navegador_linha.set_visible(nav.is_some());
                    p.procurando.set_visible(false);
                    *p.sistema.borrow_mut() = Some((s["dispositivo"].as_str().unwrap_or("").to_string(), pe["nome"].as_str().unwrap_or("").to_string(), sistema.clone()));
                    relogar_lugares();
                    if debug() {
                        eprintln!("debug primeiro uso: achado {sistema} ({} pastas, navegador {})", pastas, nav.map(|n| n["nome"].as_str().unwrap_or("").to_string()).unwrap_or_default());
                    }
                }
                None => {
                    p.procurando.set_text(tr("There is no other system on this computer's disks: nothing to bring."));
                    p.revisar.set_visible(false);
                    relogar_lugares();
                    if debug() {
                        eprintln!("debug primeiro uso: nenhum outro sistema ({})", r.err().unwrap_or_default());
                    }
                }
            }
        });
    }

    fn rodape(&self) {
        let p = self.passo.get();
        self.voltar.set_visible(p > 0);
        self.contagem.set_text(&trf("{n} of {total}", &[("n", (p + 1).to_string()), ("total", PASSOS.len().to_string())]));
        self.seguir.set_label(if p + 1 == PASSOS.len() { tr("Start") } else { tr("Continue") });
    }

    /// A step: the page comes from the side it is on, the stepper marks what is done.
    fn ir(&self, p: usize) {
        let antes = self.passo.get();
        self.passo.set(p);
        for (k, a) in self.paginas.iter().enumerate() {
            if k == p {
                a.set_visible(true);
                a.definir(Canal::X, if p == antes { 0.0 } else if p > antes { 40.0 } else { -40.0 });
                a.definir(Canal::Opacidade, if p == antes { 1.0 } else { 0.0 });
                a.ir(Canal::X, 0.0, LENTA);
                a.ir(Canal::Opacidade, 1.0, SUAVE);
            } else {
                a.set_visible(false);
            }
        }
        for (k, (n, t)) in self.marcas.iter().enumerate() {
            for c in ["feito", "atual"] {
                n.remove_css_class(c);
                t.remove_css_class(c);
            }
            let cls = if k < p { "feito" } else if k == p { "atual" } else { "" };
            if !cls.is_empty() {
                n.add_css_class(cls);
                t.add_css_class(cls);
            }
            if let Some(l) = n.first_child().and_downcast::<gtk::Label>() {
                l.set_text(&if k < p { "✓".to_string() } else { (k + 1).to_string() });
            }
        }
        let (t, s) = match p {
            0 => (tr("Hello. Want to bring your things?"), tr("Language, keyboard and account came with the installation. Choose what to bring.")),
            1 => (tr("How do you want it to look?"), tr("Choose the theme, the accent colour and the dock's style. The preview changes at once.")),
            _ => (tr("How should the agents work?"), tr("KalangoOS is made to work with agents. You decide where they think and how much they can do alone.")),
        };
        self.titulo.set_text(t);
        self.sub.set_text(s);
        self.rodape();
        relogar_lugares();
        if debug() {
            eprintln!("debug primeiro uso: passo {}", p + 1);
        }
    }

    fn continuar(self: &Rc<Self>) {
        match self.passo.get() {
            0 => {
                self.importar();
                self.ir(1);
            }
            p if p + 1 < PASSOS.len() => self.ir(p + 1),
            _ => self.comecar(),
        }
    }

    /// The copy from the other system (kalangod asks the person; it runs apart and says when it is done).
    fn importar(self: &Rc<Self>) {
        let Some((dev, pessoa, sistema)) = self.sistema.borrow().clone() else { return };
        let (arq, nav) = (self.arquivos.is_active() && self.arquivos_linha.get_visible(), self.navegador.is_active() && self.navegador_linha.get_visible());
        if self.importando.get() || !(arq || nav) {
            return;
        }
        self.importando.set(true);
        let revisar = self.revisar.is_active();
        crate::lateral::em_segundo_plano(
            move || crate::servicos::kalangod_chamar("importar.copiar", json!({"dispositivo": dev, "usuario": pessoa, "arquivos": arq, "navegador": nav})),
            move |r| {
                if debug() {
                    eprintln!("debug primeiro uso: importar {}", match &r { Ok(v) => format!("começou ({} pastas)", v["comandos"].as_array().or(v["simulado"].as_array()).map(|a| a.len()).unwrap_or(0)), Err(e) => format!("falhou: {e}") });
                }
                if r.is_ok() {
                    acompanhar(sistema, revisar);
                }
            },
        );
    }

    /// "Começar": where the agents think and how much they decide alone, then the first use is done.
    fn comecar(self: &Rc<Self>) {
        let modo = self.modos.iter().find(|(_, b)| b.is_active()).map(|(id, _)| *id).unwrap_or("hibrido");
        let autonomia = self.autonomia.iter().find(|(_, c)| c.is_active()).map(|(id, _)| *id).unwrap_or("risco");
        crate::configuracoes::salvar_ajuste("modo_agentes", json!(modo));
        let colunas: Vec<(String, String)> = AUTONOMIA.iter().find(|a| a.0 == autonomia).map(|a| a.3.iter().map(|(c, p)| (c.to_string(), p.to_string())).collect()).unwrap_or_default();
        let bin = crate::voz::agent_binary();
        let nuvem = modo != "local";
        crate::lateral::em_segundo_plano(move || {
            let mut erros = 0;
            for (c, p) in &colunas {
                if !std::process::Command::new(&bin).args(["coluna", c, p]).output().is_ok_and(|o| o.status.success()) {
                    erros += 1;
                }
            }
            let _ = std::process::Command::new(&bin).args(["memoria", "ajuste", "nuvem", if nuvem { "sim" } else { "nao" }]).output();
            erros
        }, move |erros| {
            if debug() {
                eprintln!("debug primeiro uso: agentes {modo}, autonomia {autonomia} ({erros} erros)");
            }
        });
        let digital = pendente().is_some_and(|v| v["impressao_digital"] == true);
        marcar_feito();
        if debug() {
            eprintln!("debug primeiro uso: feito");
        }
        self.janela.close();
        ABERTO.with(|a| a.borrow_mut().take());
        // "configure the fingerprint later" (installer): now, where it is done
        if digital && glib::find_program_in_path("fprintd-enroll").is_some() {
            crate::configuracoes::abrir_painel("usuarios");
        }
    }
}

fn nuvem_configurada() -> bool {
    crate::configuracoes::ajustes()["provedor_nuvem"].as_str().is_some_and(|s| !s.is_empty())
}

/// Follows the copy from the other system until it ends; then says so (and, if asked, the agent reviews it).
fn acompanhar(sistema: String, revisar: bool) {
    glib::timeout_add_seconds_local(3, move || {
        let sistema = sistema.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::servicos::kalangod_chamar("importar.estado", json!({})));
        });
        let Ok(Ok(v)) = rx.recv_timeout(std::time::Duration::from_secs(5)) else { return glib::ControlFlow::Continue };
        if v["fase"] != "terminou" {
            return glib::ControlFlow::Continue;
        }
        let tamanho = crate::configuracoes_sistema::human(v["feito"].as_u64().unwrap_or(0));
        if debug() {
            eprintln!("debug primeiro uso: importação terminou ({tamanho}, erro {})", v["erro"]);
        }
        if let Some(s) = crate::shell_atual() {
            let corpo = if v["erro"].is_null() && v["feito"].as_u64().unwrap_or(0) == 0 {
                trf("Your folders from {system} are in yours.", &[("system", sistema.clone())])
            } else if v["erro"].is_null() { trf("{size} came from {system}, into your folders.", &[("size", tamanho), ("system", sistema.clone())]) } else { trf("Part of it came from {system}; some files could not be read.", &[("system", sistema.clone())]) };
            let s2 = s.clone();
            s.notificacoes().notify_local_acoes(tr("Files"), tr("Your things arrived"), &corpo, &[("abrir", tr("Show"))], move |_| {
                s2.abrir_arquivos(glib::user_special_dir(glib::UserDirectory::Documents).as_deref());
            });
            if revisar {
                s.right_bar().ask_agent(&trf("I brought my files from {system} into my folders (Documents, Pictures, Music, Videos, Downloads). Look at what came and tell me, in a few points, what is there and what could be organised.", &[("system", sistema)]));
            }
        }
        glib::ControlFlow::Break
    });
}
