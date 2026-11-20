//! Configurações › Aparência (design "Configurações · Aparência"): the theme — dark, light or by the hour, the accent,
//! the corners, the dock's style and size, the widgets on the wallpaper — and, in its other tabs, the wallpaper and
//! the text's size. Every change is applied at once (the whole shell, live) and drawn in the preview: a small
//! desktop with the wallpaper, a window, the dock where it is.

use crate::configuracoes::{cartao, depurando, interruptor, rotulo_cartao};
use crate::configuracoes_moldura::Moldura;
use crate::conversa::ui_icon_fitted;
use crate::superficies::{caixa, rotulo, EstiloDock};
use crate::tema::{self, Tema, DESTAQUES};
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use kalango_i18n::{n_, tr};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// The tabs: (id, name).
const ABAS: &[(&str, &str)] = &[
    ("tema", n_("Theme")),
    ("cores", n_("Colours")),
    ("papel", n_("Wallpaper")),
    ("fontes", n_("Fonts")),
    ("dock", n_("Dock and multitasking")),
];

/// Which cards each tab shows.
fn cartoes_da_aba(aba: &str) -> &'static [&'static str] {
    match aba {
        "cores" => &["estilo", "destaque"],
        "papel" => &["papel"],
        "fontes" => &["texto"],
        // (the dock tab's own cards are in its main column; the panel has the windows' and the corners')
        "dock" => &["janelas", "espacos", "cantos_ativos"],
        _ => &["estilo", "destaque", "cantos", "dock", "transparencia", "arquivos_area"],
    }
}

pub struct Aparencia {
    pub moldura: Rc<Moldura>,
    previa: gtk::DrawingArea,
    cartoes: Vec<(&'static str, gtk::Widget)>,
    estilos: Vec<(&'static str, gtk::Button)>,
    destaques: Vec<(&'static str, gtk::Button, gtk::DrawingArea)>,
    cantos: gtk::Scale,
    cantos_txt: gtk::Label,
    dock_estilos: Vec<(&'static str, gtk::ToggleButton)>,
    dock_tamanho: gtk::Scale,
    dock_tamanho_txt: gtk::Label,
    transparentes: gtk::Switch,
    /// the Desktop folder's files on the desktop
    desktop_files: gtk::Switch,
    papeis: gtk::Box,
    texto: gtk::Scale,
    texto_txt: gtk::Label,
    /// filling the controls from the settings: their signals must not save them back
    lendo: Cell<bool>,
    /// a slider moving: the change goes out a moment after it stops
    pendente: RefCell<Option<glib::SourceId>>,
    /// the live preview's card (the dock tab shows its own cards in its place)
    previa_cartao: gtk::Box,
    /// the dock tab's main column — the dock, the island, the side bars —, scrolling within the window (on a short
    /// screen its cards go past the window's height)
    principal_rolagem: gtk::ScrolledWindow,
    dock_grandes: Vec<(&'static str, gtk::Button)>,
    dock_tamanho2: gtk::Scale,
    dock_tamanho2_txt: gtk::Label,
    dock_ocultar: gtk::Switch,
    dock_recentes: gtk::Switch,
    /// the taskbar's apps in the middle or on the left (only with the fixed dock)
    dock_alinhamento: Vec<(&'static str, gtk::ToggleButton)>,
    dock_alinhamento_linha: gtk::Box,
}

/// An option of a card: its name (and what it means) and its switch.
fn linha_opcao(titulo: &str, sub: &str, controle: &impl IsA<gtk::Widget>) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
    let t = caixa(Orientation::Vertical, 1, "");
    t.set_hexpand(true);
    t.set_valign(Align::Center);
    t.append(&rotulo_esq(titulo, "cfg-texto-13"));
    if !sub.is_empty() {
        let l = rotulo_esq(sub, "cfg-sub");
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l.set_max_width_chars(1);
        t.append(&l);
        r.add_css_class("alta");
    }
    r.append(&t);
    controle.set_valign(Align::Center);
    r.append(controle);
    r
}

fn rotulo_esq(t: &str, classe: &str) -> gtk::Label {
    let l = rotulo(t, classe);
    l.set_xalign(0.0);
    l
}

/// A rounded rectangle path.
fn retangulo(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.close_path();
}

fn fonte(cr: &gtk::cairo::Context, hex: &str, alfa: f64) {
    let c = gdk::RGBA::parse(tema::cor(hex)).unwrap_or(gdk::RGBA::BLACK);
    cr.set_source_rgba(f64::from(c.red()), f64::from(c.green()), f64::from(c.blue()), alfa);
}

/// A dock style's picture (the dock tab's tiles): a dark screen with the dock where that style puts it.
fn desenhar_dock_mini(cr: &gtk::cairo::Context, w: f64, h: f64, estilo: &str) {
    // the design's tile preview (277 × 110): the screen, its top bar, the dock with five apps (the first one in the
    // accent), 8 px squares 22 apart
    fonte(cr, "#101114", 1.0);
    retangulo(cr, 0.0, 0.0, w, h, 4.0);
    let _ = cr.fill();
    fonte(cr, "#1A1B21", 1.0);
    cr.rectangle(0.0, 0.0, w, 8.0);
    let _ = cr.fill();
    let app = |cr: &gtk::cairo::Context, i: u32, x: f64, y: f64| {
        fonte(cr, if i == 0 { tema::ACENTO } else { "#6A6B72" }, 1.0);
        retangulo(cr, x, y, 8.0, 8.0, 2.0);
        let _ = cr.fill();
    };
    match estilo {
        "fixa" => {
            fonte(cr, "#2A2B35", 1.0);
            cr.rectangle(0.0, h - 16.0, w, 16.0);
            let _ = cr.fill();
            for i in 0..5 {
                app(cr, i, 6.0 + f64::from(i) * 22.0, h - 13.0);
            }
        }
        "lateral" => {
            let y0 = ((h - 78.0) / 2.0 + 4.0).max(10.0);
            fonte(cr, "#2A2B35", 1.0);
            retangulo(cr, 4.0, y0, 14.0, 78.0, 4.0);
            let _ = cr.fill();
            for i in 0..5 {
                app(cr, i, 7.0, y0 + 6.0 + f64::from(i) * 14.0);
            }
        }
        _ => {
            let x0 = ((w - 120.0) / 2.0).round();
            fonte(cr, "#2A2B35", 1.0);
            retangulo(cr, x0, h - 20.0, 120.0, 14.0, 4.0);
            let _ = cr.fill();
            for i in 0..5 {
                app(cr, i, x0 + 6.0 + f64::from(i) * 22.0, h - 17.0);
            }
        }
    }
}

/// The preview: the design's small desktop (736 × 414) at this width — the wallpaper, the top bar, a window with its
/// cards and an accent button, the dock in its style, all in the theme's colours and corners.
fn desenhar_previa(cr: &gtk::cairo::Context, w: f64, h: f64, t: &Tema, d: &EstiloDock) {
    use gtk::gdk::prelude::GdkCairoContextExt;
    let s = w / 736.0;
    let k = f64::from(t.cantos) / 6.0;
    retangulo(cr, 0.0, 0.0, w, h, 4.0 * k);
    cr.clip();
    fonte(cr, "#16171B", 1.0);
    let _ = cr.paint();
    if let Some(pb) = crate::superficies::papel_de_parede(1472, 828) {
        cr.save().ok();
        cr.scale(w / f64::from(pb.width()), h / f64::from(pb.height()));
        cr.set_source_pixbuf(&pb, 0.0, 0.0);
        let _ = cr.paint();
        cr.restore().ok();
    }
    // the top bar
    fonte(cr, "#101114", 0.95);
    cr.rectangle(0.0, 0.0, w, 14.0 * s);
    let _ = cr.fill();
    // a window: its title, its side bar, three cards and a button in the accent
    let (jx, jy, jw, jh) = (150.0 * s, 70.0 * s, 420.0 * s, 250.0 * s);
    cr.save().ok();
    retangulo(cr, jx, jy, jw, jh, 4.0 * k * s.max(0.5));
    fonte(cr, "#16171B", 1.0);
    cr.fill_preserve().ok();
    cr.clip();
    fonte(cr, "#1A1B21", 1.0);
    cr.rectangle(jx, jy, jw, 22.0 * s);
    let _ = cr.fill();
    cr.rectangle(jx, jy + 22.0 * s, 110.0 * s, 228.0 * s);
    let _ = cr.fill();
    fonte(cr, "#1C1D23", 1.0);
    for i in 0..3 {
        retangulo(cr, jx + (124.0 + 98.0 * f64::from(i)) * s, jy + 40.0 * s, 86.0 * s, 70.0 * s, 3.0 * k * s);
        let _ = cr.fill();
    }
    fonte(cr, tema::ACENTO, 1.0);
    retangulo(cr, jx + 124.0 * s, jy + 128.0 * s, 80.0 * s, 18.0 * s, 3.0 * k * s);
    let _ = cr.fill();
    cr.restore().ok();
    retangulo(cr, jx + 0.5, jy + 0.5, jw - 1.0, jh - 1.0, 4.0 * k * s.max(0.5));
    fonte(cr, "#26272F", 1.0);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    // the dock: seven icons, as big as the setting asks (40 px → 14 here)
    let cores = [tema::ACENTO, "#E5C98C", "#8ED2C3", "#EFA2A6", "#7CC3DF", "#C7A7DC", "#F1DDB0"];
    let icone = 14.0 * s * f64::from(d.tamanho) / 40.0;
    let passo = icone + 9.0 * s;
    // (the design's dock: 8 before the first icon, 16 after the last)
    let comprido = 24.0 * s + icone * 7.0 + 9.0 * s * 6.0;
    let grosso = icone + 8.0 * s;
    let (dx, dy, dw, dh, horizontal) = match d.estilo.as_str() {
        "fixa" => (0.0, h - grosso, w, grosso, true),
        "lateral" => (8.0 * s, (h - comprido) / 2.0, grosso, comprido, false),
        _ => ((w - comprido) / 2.0, h - 8.0 * s - grosso, comprido, grosso, true),
    };
    fonte(cr, "#1C1D23", 0.94);
    retangulo(cr, dx, dy, dw, dh, if d.estilo == "fixa" { 0.0 } else { 4.0 * k * s });
    let _ = cr.fill();
    let inicio = if d.estilo == "fixa" { (w - comprido) / 2.0 } else if horizontal { dx } else { dy };
    for (i, c) in cores.iter().enumerate() {
        let p = inicio + 8.0 * s + passo * i as f64;
        let (x, y) = if horizontal { (p, dy + 4.0 * s) } else { (dx + 4.0 * s, p) };
        fonte(cr, c, 1.0);
        retangulo(cr, x, y, icone, icone, 3.0 * k * s);
        let _ = cr.fill();
    }
}

impl Aparencia {
    pub fn new(c: &Rc<crate::configuracoes::Configuracoes>) -> Rc<Aparencia> {
        let m = Moldura::new(tr("Appearance"), ABAS, tr("Theme settings"));
        m.sub.set_text(tr("The changes show in the live preview"));
        // the live preview
        let pc = caixa(Orientation::Vertical, 10, "cfg-cartao");
        pc.add_css_class("previa");
        pc.set_valign(Align::Start);
        let pr = caixa(Orientation::Horizontal, 5, "");
        pr.append(&ui_icon_fitted("olho-d", "#8E8F99", 16));
        pr.append(&rotulo(tr("LIVE PREVIEW"), "cfg-rotulo"));
        pc.append(&pr);
        let previa = gtk::DrawingArea::new();
        previa.set_content_width(320);
        previa.set_content_height(180);
        previa.set_hexpand(true);
        previa.set_draw_func(|_, cr, w, _| {
            // the design's 736 × 414 at most, in its proportion
            let pw = f64::from(w.min(736));
            desenhar_previa(cr, pw, (pw * 414.0 / 736.0).round(), &tema::atual(), &crate::superficies::estilo_dock());
        });
        previa.connect_resize(|p, w, _| {
            let h = ((f64::from(w.min(736))) * 414.0 / 736.0).round() as i32;
            if p.content_height() != h {
                let p = p.clone();
                glib::idle_add_local_once(move || p.set_content_height(h));
            }
        });
        pc.append(&previa);
        m.corpo.append(&pc);
        // the dock tab (design "Configurações · Dock e multitarefa"): the dock, the island and the side bars here,
        // the windows and the corners in the panel
        let principal = caixa(Orientation::Vertical, 16, "");
        let principal_rolagem = gtk::ScrolledWindow::new();
        principal_rolagem.set_hscrollbar_policy(gtk::PolicyType::Never);
        principal_rolagem.set_vexpand(true);
        principal_rolagem.set_propagate_natural_width(true);
        principal_rolagem.set_child(Some(&principal));
        principal_rolagem.set_visible(false);
        // (opened again, the tab shows from its top)
        principal_rolagem.connect_map(|r| r.vadjustment().set_value(0.0));
        m.corpo.append(&principal_rolagem);
        let cdg = cartao("");
        cdg.set_spacing(12);
        cdg.append(&rotulo_cartao("apps-pontos", tr("DOCK")));
        let tiles = caixa(Orientation::Horizontal, 12, "");
        tiles.set_homogeneous(true);
        let mut dock_grandes: Vec<(&'static str, gtk::Button)> = Vec::new();
        for (id, nome) in [("flutuante", tr("Floating")), ("fixa", tr("Fixed (taskbar)")), ("lateral", tr("Side"))] {
            let b = gtk::Button::new();
            b.add_css_class("cfg-dock-estilo");
            // (the design's tile: 8 of padding and of gap, a 110 px preview, the radio's row of 26)
            let col = caixa(Orientation::Vertical, 8, "");
            let mini = gtk::DrawingArea::new();
            mini.set_content_height(110);
            mini.set_hexpand(true);
            mini.set_draw_func(move |_, cr, w, h| desenhar_dock_mini(cr, f64::from(w), f64::from(h), id));
            col.append(&mini);
            let r = caixa(Orientation::Horizontal, 8, "");
            r.set_margin_top(4);
            r.set_margin_bottom(4);
            r.set_margin_start(4);
            r.append(&caixa(Orientation::Horizontal, 0, "cfg-radio"));
            r.append(&rotulo(nome, "cfg-texto-13"));
            col.append(&r);
            b.set_child(Some(&col));
            tiles.append(&b);
            dock_grandes.push((id, b));
        }
        cdg.append(&tiles);
        let dock_tamanho2_txt = rotulo("", "cfg-valor-fraco");
        let dock_tamanho2 = gtk::Scale::with_range(Orientation::Horizontal, 26.0, 56.0, 2.0);
        dock_tamanho2.add_css_class("cfg-escala");
        dock_tamanho2.set_draw_value(false);
        dock_tamanho2.set_size_request(200, -1);
        let tam = caixa(Orientation::Horizontal, 10, "");
        tam.append(&dock_tamanho2);
        tam.append(&dock_tamanho2_txt);
        let opcoes_dock = caixa(Orientation::Vertical, 2, "");
        opcoes_dock.append(&linha_opcao(tr("Icon size"), "", &tam));
        let alin = caixa(Orientation::Horizontal, 2, "cfg-segmentado");
        alin.add_css_class("escuro");
        alin.set_valign(Align::Center);
        let mut dock_alinhamento: Vec<(&'static str, gtk::ToggleButton)> = Vec::new();
        for (id, nome) in [("centro", tr("Center")), ("esquerda", tr("On the left"))] {
            let b = gtk::ToggleButton::with_label(nome);
            b.add_css_class("cfg-seg");
            if let Some((_, p)) = dock_alinhamento.first() {
                b.set_group(Some(p));
            }
            alin.append(&b);
            dock_alinhamento.push((id, b));
        }
        let dock_alinhamento_linha = linha_opcao(tr("Taskbar alignment"), "", &alin);
        opcoes_dock.append(&dock_alinhamento_linha);
        let dock_ocultar = interruptor();
        opcoes_dock.append(&linha_opcao(tr("Hide automatically"), tr("comes back when the pointer touches the edge"), &dock_ocultar));
        let dock_recentes = interruptor();
        opcoes_dock.append(&linha_opcao(tr("Show recent and suggested apps"), "", &dock_recentes));
        cdg.append(&opcoes_dock);
        principal.append(&cdg);
        let controles = m.painel.clone();
        let mut cartoes: Vec<(&'static str, gtk::Widget)> = Vec::new();
        // style
        let ce = cartao("");
        ce.append(&rotulo_esq(tr("STYLE"), "cfg-rotulo"));
        let opcoes = caixa(Orientation::Horizontal, 10, "");
        opcoes.set_homogeneous(true);
        let mut estilos = Vec::new();
        for (id, nome) in [("claro", tr("Light")), ("escuro", tr("Dark")), ("automatico", tr("Automatic"))] {
            let b = gtk::Button::new();
            b.add_css_class("cfg-estilo");
            let col = caixa(Orientation::Vertical, 6, "");
            let amostra = caixa(Orientation::Horizontal, 0, "cfg-amostra");
            amostra.add_css_class(id);
            amostra.set_size_request(-1, 64);
            col.append(&amostra);
            col.append(&rotulo(nome, "cfg-estilo-nome"));
            b.set_child(Some(&col));
            if id == "automatico" {
                b.set_tooltip_text(Some(tr("Light from 7h to 19h, dark at night")));
            }
            opcoes.append(&b);
            estilos.push((id, b));
        }
        ce.append(&opcoes);
        controles.append(&ce);
        cartoes.push(("estilo", ce.upcast()));
        // accent
        let cd = cartao("");
        cd.append(&rotulo_esq(tr("ACCENT COLOUR"), "cfg-rotulo"));
        let cores = caixa(Orientation::Horizontal, 10, "");
        let mut destaques = Vec::new();
        let nomes = [tr("Lavender"), tr("Teal"), tr("Blue"), tr("Coral"), tr("Gold"), tr("Purple"), tr("Cream")];
        for ((id, escuro, token), nome) in DESTAQUES.iter().zip(nomes) {
            let b = gtk::Button::new();
            b.add_css_class("cfg-cor");
            b.set_tooltip_text(Some(nome));
            let da = gtk::DrawingArea::new();
            da.set_content_width(28);
            da.set_content_height(28);
            let (id_, escuro, token) = (*id, *escuro, *token);
            da.set_draw_func(move |_, cr, w, h| {
                let t = tema::atual();
                // the colour this swatch gives in the theme as it is now
                let cor = if tema::e_claro() { tema::token_claro(token).unwrap_or_else(|| escuro.to_string()) } else { escuro.to_string() };
                let c = gdk::RGBA::parse(&cor).unwrap_or(gdk::RGBA::WHITE);
                cr.set_source_rgba(f64::from(c.red()), f64::from(c.green()), f64::from(c.blue()), 1.0);
                retangulo(cr, 0.0, 0.0, f64::from(w), f64::from(h), 8.0);
                let _ = cr.fill();
                if t.destaque == id_ {
                    let borda = if tema::e_claro() { "#322153" } else { "#FFFFFF" };
                    let c = gdk::RGBA::parse(borda).unwrap_or(gdk::RGBA::WHITE);
                    cr.set_source_rgba(f64::from(c.red()), f64::from(c.green()), f64::from(c.blue()), 1.0);
                    cr.set_line_width(2.0);
                    retangulo(cr, 1.0, 1.0, f64::from(w) - 2.0, f64::from(h) - 2.0, 7.0);
                    let _ = cr.stroke();
                }
            });
            b.set_child(Some(&da));
            cores.append(&b);
            destaques.push((*id, b, da));
        }
        cd.append(&cores);
        controles.append(&cd);
        cartoes.push(("destaque", cd.upcast()));
        // corners
        let cc = cartao("");
        let ct = caixa(Orientation::Horizontal, 0, "");
        let l = rotulo_esq(tr("ROUNDING"), "cfg-rotulo");
        l.set_hexpand(true);
        ct.append(&l);
        let cantos_txt = rotulo("", "cfg-valor-pequeno");
        ct.append(&cantos_txt);
        cc.append(&ct);
        let cantos = gtk::Scale::with_range(Orientation::Horizontal, 0.0, 20.0, 1.0);
        cantos.add_css_class("cfg-escala");
        cantos.set_draw_value(false);
        cc.append(&cantos);
        controles.append(&cc);
        cartoes.push(("cantos", cc.upcast()));
        // the dock
        let ck = cartao("");
        ck.append(&rotulo_cartao("apps-pontos", tr("DOCK")));
        let seg = caixa(Orientation::Horizontal, 2, "cfg-segmentado");
        seg.set_homogeneous(true);
        let mut dock_estilos: Vec<(&'static str, gtk::ToggleButton)> = Vec::new();
        for (id, nome) in [("flutuante", tr("Floating")), ("fixa", tr("Fixed (bar)")), ("lateral", tr("Side"))] {
            let b = gtk::ToggleButton::with_label(nome);
            b.add_css_class("cfg-seg");
            if let Some((_, p)) = dock_estilos.first() {
                b.set_group(Some(p));
            }
            seg.append(&b);
            dock_estilos.push((id, b));
        }
        ck.append(&seg);
        let tr_ = caixa(Orientation::Horizontal, 0, "");
        let l = rotulo_esq(tr("Icon size"), "cfg-sub");
        l.set_hexpand(true);
        tr_.append(&l);
        let dock_tamanho_txt = rotulo("", "cfg-valor-pequeno");
        tr_.append(&dock_tamanho_txt);
        ck.append(&tr_);
        let dock_tamanho = gtk::Scale::with_range(Orientation::Horizontal, 26.0, 56.0, 2.0);
        dock_tamanho.add_css_class("cfg-escala");
        dock_tamanho.set_draw_value(false);
        ck.append(&dock_tamanho);
        controles.append(&ck);
        cartoes.push(("dock", ck.upcast()));
        // the island: its live activities, and whether the pointer resting on it expands it (the expanded island)
        let ci = cartao("");
        ci.set_spacing(2);
        ci.append(&rotulo_cartao("spark", tr("ISLAND")));
        let ao_vivo = interruptor();
        ci.append(&linha_opcao(tr("Live activities in the island"), tr("agents, music, copies, timers"), &ao_vivo));
        let ao_passar = interruptor();
        ci.append(&linha_opcao(tr("Expand when the pointer rests on it"), "", &ao_passar));
        let lendo_ilha = Rc::new(Cell::new(false));
        {
            let (v, p, l) = (ao_vivo.downgrade(), ao_passar.downgrade(), lendo_ilha.clone());
            // what is kept now, each time the card shows (the notifications page changes the same choice)
            ci.connect_map(move |_| {
                l.set(true);
                if let (Some(v), Some(p)) = (v.upgrade(), p.upgrade()) {
                    let ilha = crate::notification_policy::read().in_island;
                    v.set_active(ilha);
                    v.set_state(ilha);
                    let passar = crate::island_card::expands_on_hover();
                    p.set_active(passar);
                    p.set_state(passar);
                }
                l.set(false);
            });
        }
        let l = lendo_ilha.clone();
        ao_vivo.connect_state_set(move |_, on| {
            if !l.get() {
                crate::notification_policy::update(|x| x.in_island = on);
                if depurando() {
                    eprintln!("debug aparencia: atividades na ilha {on}");
                }
            }
            glib::Propagation::Proceed
        });
        let l = lendo_ilha.clone();
        ao_passar.connect_state_set(move |_, on| {
            if !l.get() {
                crate::configuracoes::salvar_ajuste("ilha_expandir_ao_passar", serde_json::json!(on));
                if depurando() {
                    eprintln!("debug aparencia: expandir a ilha ao passar o ponteiro {on}");
                }
            }
            glib::Propagation::Proceed
        });
        crate::configuracoes::registrar_posicao(c, "cfg aparencia ilha ao vivo", ao_vivo.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg aparencia ilha passar", ao_passar.upcast_ref());
        principal.append(&ci);
        // the side bars: opened by the screen's edges, pinned on wide screens, their width
        let cb = cartao("");
        cb.set_spacing(2);
        cb.append(&rotulo_cartao("painel-direita", tr("SIDE BARS")));
        let na_borda = interruptor();
        cb.append(&linha_opcao(tr("Open when the pointer rests on the edge"), tr("left: Today · right: Notices, Controls and Agent"), &na_borda));
        let fixas = interruptor();
        cb.append(&linha_opcao(tr("Pinned by default on wide screens"), tr("from 2560 px wide"), &fixas));
        // (the design's dark segmented control: each segment as wide as its name)
        let larg = caixa(Orientation::Horizontal, 2, "cfg-segmentado");
        larg.add_css_class("escuro");
        larg.set_valign(Align::Center);
        let mut larguras: Vec<(&'static str, gtk::ToggleButton)> = Vec::new();
        for (id, nome) in [("estreita", tr("Narrow")), ("padrao", tr("Standard")), ("larga", tr("Wide"))] {
            let b = gtk::ToggleButton::with_label(nome);
            b.add_css_class("cfg-seg");
            if let Some((_, p)) = larguras.first() {
                b.set_group(Some(p));
            }
            larg.append(&b);
            larguras.push((id, b));
        }
        cb.append(&linha_opcao(tr("Width"), "", &larg));
        principal.append(&cb);
        // the windows (the panel)
        let cj = cartao("");
        cj.set_spacing(2);
        cj.append(&rotulo_cartao("layout", tr("WINDOWS")));
        let encaixe = interruptor();
        cj.append(&linha_opcao(tr("Snap when dragged to the edges"), "", &encaixe));
        let sugere = interruptor();
        cj.append(&linha_opcao(tr("The agent suggests layouts"), tr("based on the open apps"), &sugere));
        let lados = caixa(Orientation::Horizontal, 2, "cfg-segmentado");
        lados.add_css_class("escuro");
        lados.set_valign(Align::Center);
        let mut botoes_lado: Vec<(bool, gtk::ToggleButton)> = Vec::new();
        for (esq, nome) in [(false, tr("On the right")), (true, tr("On the left"))] {
            let b = gtk::ToggleButton::with_label(nome);
            b.add_css_class("cfg-seg");
            if let Some((_, p)) = botoes_lado.first() {
                b.set_group(Some(p));
            }
            lados.append(&b);
            botoes_lado.push((esq, b));
        }
        cj.append(&linha_opcao(tr("Window buttons"), "", &lados));
        let foco = interruptor();
        cj.append(&linha_opcao(tr("Focus follows the pointer"), "", &foco));
        controles.append(&cj);
        cartoes.push(("janelas", cj.upcast()));
        // the spaces (design "Espaços por intenção"): their rules, and switching by routine
        let ce2 = cartao("");
        ce2.set_spacing(2);
        ce2.append(&rotulo_cartao("camadas", tr("SPACES")));
        let intencao = interruptor();
        ce2.append(&linha_opcao(tr("Spaces by intention"), tr("each space remembers windows, agents and focus"), &intencao));
        let por_rotina = interruptor();
        ce2.append(&linha_opcao(tr("Switch spaces by routine"), tr("e.g. Relax at 19h"), &por_rotina));
        {
            let (i, r) = (intencao.downgrade(), por_rotina.downgrade());
            let l = Rc::new(Cell::new(false));
            let l2 = l.clone();
            ce2.connect_map(move |_| {
                l2.set(true);
                for (sw, v) in [(i.upgrade(), crate::spaces::intentions_on()), (r.upgrade(), crate::spaces::by_routine())] {
                    if let Some(sw) = sw {
                        sw.set_active(v);
                        sw.set_state(v);
                    }
                }
                l2.set(false);
            });
            let l3 = l.clone();
            intencao.connect_state_set(move |_, on| {
                if !l3.get() {
                    crate::spaces::set_intentions(on);
                }
                glib::Propagation::Proceed
            });
            por_rotina.connect_state_set(move |_, on| {
                if !l.get() {
                    crate::spaces::set_by_routine(on);
                }
                glib::Propagation::Proceed
            });
        }
        crate::configuracoes::registrar_posicao(c, "cfg aparencia espacos intencao", intencao.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg aparencia espacos rotina", por_rotina.upcast_ref());
        controles.append(&ce2);
        cartoes.push(("espacos", ce2.upcast()));
        // the active corners
        let cc2 = cartao("");
        cc2.append(&rotulo_cartao("ponteiro", tr("ACTIVE CORNERS")));
        let grade_cantos = gtk::Grid::new();
        grade_cantos.set_row_spacing(8);
        grade_cantos.set_column_spacing(8);
        grade_cantos.set_column_homogeneous(true);
        let nomes_acoes: Vec<&str> = crate::edges::CORNER_ACTIONS.iter().map(|(_, n)| kalango_i18n::tr(n)).collect();
        let mut seletores_cantos: Vec<(&'static str, gtk::DropDown)> = Vec::new();
        for (i, (canto, _)) in crate::edges::CORNERS.iter().enumerate() {
            let d = gtk::DropDown::new(Some(gtk::StringList::new(&nomes_acoes)), None::<gtk::Expression>);
            d.add_css_class("cfg-seletor");
            d.set_hexpand(true);
            grade_cantos.attach(&d, (i % 2) as i32, (i / 2) as i32, 1, 1);
            seletores_cantos.push((canto, d));
        }
        cc2.append(&grade_cantos);
        controles.append(&cc2);
        cartoes.push(("cantos_ativos", cc2.upcast()));
        // filled each time the cards show (what is kept now), and kept on each change
        let lendo_dock = Rc::new(Cell::new(false));
        {
            let l = lendo_dock.clone();
            let (nb, fx, fo, en, sg) = (na_borda.downgrade(), fixas.downgrade(), foco.downgrade(), encaixe.downgrade(), sugere.downgrade());
            let larguras2: Vec<(&'static str, glib::WeakRef<gtk::ToggleButton>)> = larguras.iter().map(|(i, b)| (*i, b.downgrade())).collect();
            let lados2: Vec<(bool, glib::WeakRef<gtk::ToggleButton>)> = botoes_lado.iter().map(|(e, b)| (*e, b.downgrade())).collect();
            let cantos2: Vec<(&'static str, glib::WeakRef<gtk::DropDown>)> = seletores_cantos.iter().map(|(c, d)| (*c, d.downgrade())).collect();
            let encher = move || {
                l.set(true);
                let a = crate::configuracoes::ajustes();
                if let Some(x) = nb.upgrade() {
                    let v = crate::edges::edges_open_bars();
                    x.set_active(v);
                    x.set_state(v);
                }
                if let Some(x) = fx.upgrade() {
                    let v = a["barras_fixas_largas"].as_bool().unwrap_or(false);
                    x.set_active(v);
                    x.set_state(v);
                }
                if let Some(x) = fo.upgrade() {
                    let v = crate::window_prefs::focus_follows();
                    x.set_active(v);
                    x.set_state(v);
                }
                if let Some(x) = en.upgrade() {
                    let v = crate::window_prefs::edge_snap();
                    x.set_active(v);
                    x.set_state(v);
                }
                if let Some(x) = sg.upgrade() {
                    let v = crate::layout_habits::enabled();
                    x.set_active(v);
                    x.set_state(v);
                }
                let atual = a["barras_largura"].as_str().unwrap_or("padrao").to_string();
                for (id, b) in &larguras2 {
                    if let Some(b) = b.upgrade() {
                        b.set_active(*id == atual);
                    }
                }
                let esq = crate::window_prefs::buttons_left();
                for (e, b) in &lados2 {
                    if let Some(b) = b.upgrade() {
                        b.set_active(*e == esq);
                    }
                }
                for (canto, d) in &cantos2 {
                    if let Some(d) = d.upgrade() {
                        let acao = crate::edges::corner_action(canto);
                        d.set_selected(crate::edges::CORNER_ACTIONS.iter().position(|(id, _)| *id == acao).unwrap_or(0) as u32);
                    }
                }
                l.set(false);
            };
            principal.connect_map(move |_| encher());
        }
        let l = lendo_dock.clone();
        na_borda.connect_state_set(move |_, on| {
            if !l.get() {
                crate::configuracoes::salvar_ajuste("barras_na_borda", serde_json::json!(on));
                crate::edges::apply();
            }
            glib::Propagation::Proceed
        });
        let l = lendo_dock.clone();
        fixas.connect_state_set(move |_, on| {
            if !l.get() {
                crate::configuracoes::salvar_ajuste("barras_fixas_largas", serde_json::json!(on));
                // a wide screen now: pinned at once
                if on && crate::superficies::tamanho_da_saida().is_some_and(|(w, _)| w >= 2560) {
                    if let Some(s) = crate::shell_atual() {
                        s.right_bar().pin_open();
                        s.today_bar().pin_open();
                    }
                }
                if depurando() {
                    eprintln!("debug aparencia: barras fixas em telas largas {on}");
                }
            }
            glib::Propagation::Proceed
        });
        for (id, b) in &larguras {
            let (l, id) = (lendo_dock.clone(), *id);
            b.connect_toggled(move |b| {
                if b.is_active() && !l.get() {
                    crate::configuracoes::salvar_ajuste("barras_largura", serde_json::json!(id));
                    if let Some(s) = crate::shell_atual() {
                        s.right_bar().width_changed();
                        s.today_bar().width_changed();
                    }
                    if depurando() {
                        eprintln!("debug aparencia: largura das barras {id} ({} px)", crate::lateral::largura());
                    }
                }
            });
        }
        for (esq, b) in &botoes_lado {
            let (l, esq) = (lendo_dock.clone(), *esq);
            b.connect_toggled(move |b| {
                if b.is_active() && !l.get() {
                    crate::window_prefs::set_buttons_left(esq);
                }
            });
        }
        let l = lendo_dock.clone();
        encaixe.connect_state_set(move |_, on| {
            if !l.get() {
                crate::window_prefs::set_edge_snap(on);
            }
            glib::Propagation::Proceed
        });
        crate::configuracoes::registrar_posicao(c, "cfg aparencia encaixe", encaixe.upcast_ref());
        let l = lendo_dock.clone();
        sugere.connect_state_set(move |_, on| {
            if !l.get() {
                crate::layout_habits::set_enabled(on);
            }
            glib::Propagation::Proceed
        });
        crate::configuracoes::registrar_posicao(c, "cfg aparencia sugere layouts", sugere.upcast_ref());
        let l = lendo_dock.clone();
        foco.connect_state_set(move |_, on| {
            if !l.get() {
                crate::window_prefs::set_focus_follows(on);
            }
            glib::Propagation::Proceed
        });
        for (canto, d) in &seletores_cantos {
            let (l, canto) = (lendo_dock.clone(), *canto);
            d.connect_selected_notify(move |d| {
                if !l.get() {
                    if let Some((id, _)) = crate::edges::CORNER_ACTIONS.get(d.selected() as usize) {
                        crate::edges::set_corner_action(canto, id);
                    }
                }
            });
        }
        crate::configuracoes::registrar_posicao(c, "cfg aparencia borda", na_borda.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg aparencia larga", larguras[2].1.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg aparencia canto sup_dir", seletores_cantos[1].1.upcast_ref());
        // widgets
        let cw = cartao("");
        let wt = caixa(Orientation::Horizontal, 0, "");
        let l = rotulo_esq(tr("TRANSPARENT WIDGETS"), "cfg-rotulo");
        l.set_hexpand(true);
        l.set_valign(Align::Center);
        wt.append(&l);
        let transparentes = interruptor();
        wt.append(&transparentes);
        cw.append(&wt);
        controles.append(&cw);
        cartoes.push(("transparencia", cw.upcast()));
        // the Desktop folder's files as icons on the desktop
        let ca = cartao("");
        let at = caixa(Orientation::Horizontal, 0, "");
        let l = rotulo_esq(tr("DESKTOP FILES"), "cfg-rotulo");
        l.set_hexpand(true);
        l.set_valign(Align::Center);
        at.append(&l);
        let desktop_files = interruptor();
        at.append(&desktop_files);
        ca.append(&at);
        let sub = rotulo_esq(tr("The files and folders of the Desktop folder, as icons beside the widgets."), "cfg-sub");
        sub.set_wrap(true);
        sub.set_max_width_chars(24);
        ca.append(&sub);
        controles.append(&ca);
        cartoes.push(("arquivos_area", ca.upcast()));
        // the wallpaper (its tab)
        let cp = cartao("");
        cp.append(&rotulo_esq(tr("WALLPAPER"), "cfg-rotulo"));
        let papeis = caixa(Orientation::Vertical, 10, "");
        cp.append(&papeis);
        controles.append(&cp);
        cartoes.push(("papel", cp.upcast()));
        // the text (its tab)
        let cx = cartao("");
        let xt = caixa(Orientation::Horizontal, 0, "");
        let l = rotulo_esq(tr("TEXT SIZE"), "cfg-rotulo");
        l.set_hexpand(true);
        xt.append(&l);
        let texto_txt = rotulo("", "cfg-valor-pequeno");
        xt.append(&texto_txt);
        cx.append(&xt);
        let texto = gtk::Scale::with_range(Orientation::Horizontal, 80.0, 150.0, 5.0);
        texto.add_css_class("cfg-escala");
        texto.set_draw_value(false);
        cx.append(&texto);
        let amostra = rotulo_esq(tr("The quick brown fox jumps over the lazy dog."), "cfg-texto");
        amostra.set_wrap(true);
        cx.append(&amostra);
        controles.append(&cx);
        cartoes.push(("texto", cx.upcast()));
        let a = Rc::new(Aparencia { moldura: m, previa, cartoes, estilos, destaques, cantos, cantos_txt, dock_estilos, dock_tamanho,
            dock_tamanho_txt, transparentes, desktop_files, papeis, texto, texto_txt, lendo: Cell::new(false), pendente: RefCell::default(),
            previa_cartao: pc, principal_rolagem, dock_grandes, dock_tamanho2, dock_tamanho2_txt, dock_ocultar, dock_recentes, dock_alinhamento, dock_alinhamento_linha });
        a.ligar(c);
        a.aba("tema");
        a.preencher();
        let w = Rc::downgrade(&a);
        tema::ao_mudar(move || if let Some(a) = w.upgrade() { a.preencher() });
        a
    }

    fn ligar(self: &Rc<Self>, c: &Rc<crate::configuracoes::Configuracoes>) {
        let w = Rc::downgrade(self);
        self.moldura.ao_trocar_aba(move |id| if let Some(a) = w.upgrade() { a.aba(id) });
        for (id, b) in &self.estilos {
            let (w, id) = (Rc::downgrade(self), *id);
            b.connect_clicked(move |_| if let Some(a) = w.upgrade() { a.mudar(|t| t.estilo = id.into()) });
        }
        for (id, b, _) in &self.destaques {
            let (w, id) = (Rc::downgrade(self), *id);
            b.connect_clicked(move |_| if let Some(a) = w.upgrade() { a.mudar(|t| t.destaque = id.into()) });
        }
        let w = Rc::downgrade(self);
        self.cantos.connect_value_changed(move |s| {
            let Some(a) = w.upgrade() else { return };
            let v = s.value().round() as u32;
            a.cantos_txt.set_text(&format!("{} · {v} px", tema::nome_dos_cantos(v)));
            a.depois(move |a| a.mudar(|t| t.cantos = v));
        });
        let w = Rc::downgrade(self);
        self.desktop_files.connect_state_set(move |_, on| {
            if let Some(a) = w.upgrade() {
                a.mudar(|t| t.desktop_files = on);
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.transparentes.connect_state_set(move |_, on| {
            if let Some(a) = w.upgrade() {
                a.mudar(|t| t.widgets_transparentes = on);
            }
            glib::Propagation::Proceed
        });
        for (id, b) in &self.dock_estilos {
            let (w, id) = (Rc::downgrade(self), *id);
            b.connect_toggled(move |b| {
                if b.is_active() {
                    if let Some(a) = w.upgrade() {
                        a.mudar_dock(|d| d.estilo = id.into());
                    }
                }
            });
        }
        for (id, b) in &self.dock_grandes {
            let (w, id) = (Rc::downgrade(self), *id);
            b.connect_clicked(move |_| {
                if let Some(a) = w.upgrade() {
                    a.mudar_dock(|d| d.estilo = id.into());
                    a.preencher();
                }
            });
        }
        let w = Rc::downgrade(self);
        self.dock_tamanho2.connect_value_changed(move |s| {
            let Some(a) = w.upgrade() else { return };
            let v = (s.value().round() as i32 / 2) * 2;
            a.dock_tamanho2_txt.set_text(&format!("{v} px"));
            a.depois(move |a| a.mudar_dock(|d| d.tamanho = v));
        });
        if let Some((_, b)) = self.dock_grandes.get(1) {
            crate::configuracoes::registrar_posicao(c, "cfg aparencia dock fixa", b.upcast_ref());
        }
        if let Some((_, b)) = self.dock_grandes.first() {
            crate::configuracoes::registrar_posicao(c, "cfg aparencia dock flutuante", b.upcast_ref());
        }
        let w = Rc::downgrade(self);
        self.dock_ocultar.connect_state_set(move |_, on| {
            if let Some(a) = w.upgrade() {
                a.mudar_dock(|d| d.ocultar = on);
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.dock_recentes.connect_state_set(move |_, on| {
            if let Some(a) = w.upgrade() {
                a.mudar_dock(|d| d.recentes = on);
            }
            glib::Propagation::Proceed
        });
        for (id, b) in &self.dock_alinhamento {
            let (w, id) = (Rc::downgrade(self), *id);
            b.connect_toggled(move |b| {
                if b.is_active() {
                    if let Some(a) = w.upgrade() {
                        a.mudar_dock(|d| d.alinhamento = id.into());
                    }
                }
            });
            crate::configuracoes::registrar_posicao(c, &format!("cfg aparencia alinhamento {id}"), b.upcast_ref());
        }
        crate::configuracoes::registrar_posicao(c, "cfg aparencia dock ocultar", self.dock_ocultar.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg aparencia dock recentes", self.dock_recentes.upcast_ref());
        let w = Rc::downgrade(self);
        self.dock_tamanho.connect_value_changed(move |s| {
            let Some(a) = w.upgrade() else { return };
            let v = (s.value().round() as i32 / 2) * 2;
            a.dock_tamanho_txt.set_text(&format!("{v} px"));
            a.depois(move |a| a.mudar_dock(|d| d.tamanho = v));
        });
        let w = Rc::downgrade(self);
        self.texto.connect_value_changed(move |s| {
            let Some(a) = w.upgrade() else { return };
            let v = s.value().round();
            a.texto_txt.set_text(&format!("{v}%"));
            a.depois(move |a| a.mudar(|t| t.texto = v / 100.0));
        });
        for (nome, b) in [("cfg aparencia escuro", &self.estilos[1].1), ("cfg aparencia claro", &self.estilos[0].1), ("cfg aparencia teal", &self.destaques[1].1),
            ("cfg aparencia lavanda", &self.destaques[0].1),
            ("cfg aparencia lateral", self.dock_estilos[2].1.upcast_ref()), ("cfg aparencia flutuante", self.dock_estilos[0].1.upcast_ref())] {
            crate::configuracoes::registrar_posicao(c, nome, b.upcast_ref());
        }
        crate::configuracoes::registrar_posicao(c, "cfg aparencia transparentes", self.transparentes.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg aparencia arquivos area", self.desktop_files.upcast_ref());
    }

    pub fn compactar(&self, c: bool) {
        self.moldura.compactar(c);
    }

    /// Runs `f` 150 ms after the last call (a slider being dragged).
    fn depois(self: &Rc<Self>, f: impl FnOnce(&Rc<Self>) + 'static) {
        if self.lendo.get() {
            return;
        }
        if let Some(s) = self.pendente.borrow_mut().take() {
            s.remove();
        }
        let w = Rc::downgrade(self);
        *self.pendente.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(150), move || {
            if let Some(a) = w.upgrade() {
                a.pendente.borrow_mut().take();
                f(&a);
            }
        }));
    }

    /// Changes the theme, keeps it and applies it to the whole shell.
    fn mudar(&self, f: impl FnOnce(&mut Tema)) {
        if self.lendo.get() {
            return;
        }
        let mut t = tema::ler();
        f(&mut t);
        tema::salvar(&t);
        tema::aplicar(&t);
        self.previa.queue_draw();
        self.subtitulo();
        if depurando() {
            eprintln!("debug configuracoes: tema {} {} cantos {} widgets {} texto {}", t.estilo, t.destaque, t.cantos, t.widgets_transparentes, t.texto);
        }
    }

    fn mudar_dock(&self, f: impl FnOnce(&mut EstiloDock)) {
        if self.lendo.get() {
            return;
        }
        let mut d = crate::superficies::estilo_dock();
        f(&mut d);
        self.dock_alinhamento_linha.set_sensitive(d.estilo == "fixa");
        crate::superficies::salvar_estilo_dock(&d);
        crate::superficies::refazer_dock();
        self.previa.queue_draw();
        self.subtitulo();
    }

    /// The page's subtitle: on the dock tab, the theme and the dock as they are (design: "Tema escuro · dock
    /// flutuante"); on the others, that the preview is live.
    fn subtitulo(&self) {
        if !self.principal_rolagem.is_visible() {
            self.moldura.sub.set_text(tr("The changes show in the live preview"));
            return;
        }
        let tema = match tema::ler().estilo.as_str() {
            "claro" => tr("Light theme"),
            "automatico" => tr("Automatic theme"),
            _ => tr("Dark theme"),
        };
        let dock = match crate::superficies::estilo_dock().estilo.as_str() {
            "fixa" => tr("fixed dock"),
            "lateral" => tr("side dock"),
            _ => tr("floating dock"),
        };
        self.moldura.sub.set_text(&format!("{tema} · {dock}"));
    }

    /// A tab: the panel shows its cards.
    pub(crate) fn aba(&self, id: &str) {
        let mostrar = cartoes_da_aba(id);
        for (nome, w) in &self.cartoes {
            w.set_visible(mostrar.contains(nome));
        }
        // the dock tab: its cards where the preview is, and the panel is "Janelas e espaços"
        let dock = id == "dock";
        self.previa_cartao.set_visible(!dock);
        self.principal_rolagem.set_visible(dock);
        self.moldura.painel_titulo.set_text(if dock { tr("Windows and spaces") } else { tr("Theme settings") });
        self.subtitulo();
        // the dock tab's window is taller in the design (its three cards without scrolling)
        if let Some(c) = crate::shell_atual().and_then(|s| s.configuracoes_atual()) {
            c.pedir_altura(if dock { 943 } else { crate::configuracoes::altura_da_pagina("aparencia") });
        }
    }

    /// The controls as the settings are now.
    fn preencher(self: &Rc<Self>) {
        self.lendo.set(true);
        let t = tema::ler();
        for (id, b) in &self.estilos {
            if t.estilo == *id {
                b.add_css_class("escolhido");
            } else {
                b.remove_css_class("escolhido");
            }
        }
        for (_, _, da) in &self.destaques {
            da.queue_draw();
        }
        self.cantos.set_value(f64::from(t.cantos));
        self.cantos_txt.set_text(&format!("{} · {} px", tema::nome_dos_cantos(t.cantos), t.cantos));
        self.transparentes.set_active(t.widgets_transparentes);
        self.transparentes.set_state(t.widgets_transparentes);
        self.desktop_files.set_active(t.desktop_files);
        self.desktop_files.set_state(t.desktop_files);
        let d = crate::superficies::estilo_dock();
        for (id, b) in &self.dock_estilos {
            if d.estilo == *id {
                b.set_active(true);
            }
        }
        self.dock_tamanho.set_value(f64::from(d.tamanho));
        self.dock_tamanho_txt.set_text(&format!("{} px", d.tamanho));
        self.dock_tamanho2.set_value(f64::from(d.tamanho));
        self.dock_tamanho2_txt.set_text(&format!("{} px", d.tamanho));
        for (id, b) in &self.dock_grandes {
            if d.estilo == *id { b.add_css_class("escolhido") } else { b.remove_css_class("escolhido") }
        }
        for (id, b) in &self.dock_alinhamento {
            if d.alinhamento == *id {
                b.set_active(true);
            }
        }
        self.dock_alinhamento_linha.set_sensitive(d.estilo == "fixa");
        for (s, v) in [(&self.dock_ocultar, d.ocultar), (&self.dock_recentes, d.recentes)] {
            s.set_active(v);
            s.set_state(v);
        }
        self.texto.set_value((t.texto * 100.0).round());
        self.texto_txt.set_text(&format!("{}%", (t.texto * 100.0).round()));
        self.preencher_papeis(&t);
        self.previa.queue_draw();
        self.lendo.set(false);
    }

    /// The wallpapers: KalangoOS's scene (light or dark with the theme), the pictures chosen before, and "Choose a
    /// picture…".
    fn preencher_papeis(self: &Rc<Self>, t: &Tema) {
        while let Some(x) = self.papeis.first_child() {
            self.papeis.remove(&x);
        }
        let grade = gtk::FlowBox::new();
        grade.set_selection_mode(gtk::SelectionMode::None);
        grade.add_css_class("cfg-grade");
        grade.set_column_spacing(10);
        grade.set_row_spacing(10);
        // (as many pictures per row as fit — two on a narrow page, more on a wide one)
        grade.set_min_children_per_line(2);
        grade.set_max_children_per_line(8);
        grade.set_homogeneous(true);
        grade.set_hexpand(true);
        let mut opcoes: Vec<(Option<String>, String)> = vec![(None, tr("KalangoOS scene").to_string())];
        for p in &t.papeis {
            let nome = std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            opcoes.push((Some(p.clone()), nome));
        }
        for (caminho, nome) in opcoes {
            let b = gtk::Button::new();
            b.add_css_class("cfg-estilo");
            if t.papel == caminho {
                b.add_css_class("escolhido");
            }
            let col = caixa(Orientation::Vertical, 6, "");
            let mini = gtk::DrawingArea::new();
            mini.set_content_width(122);
            mini.set_content_height(64);
            let c2 = caminho.clone();
            mini.set_draw_func(move |_, cr, w, h| {
                use gtk::gdk::prelude::GdkCairoContextExt;
                retangulo(cr, 0.0, 0.0, f64::from(w), f64::from(h), 4.0);
                cr.clip();
                let p = c2.clone().map(std::path::PathBuf::from).or_else(|| {
                    if tema::papel_escolhido().is_none() { crate::papel::para(1920, 1080) } else { None }
                });
                let pb = p.and_then(|p| gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(p, w * 2, -1, true).ok());
                match pb {
                    Some(pb) => {
                        let k = (f64::from(w) / f64::from(pb.width())).max(f64::from(h) / f64::from(pb.height()));
                        cr.scale(k, k);
                        cr.set_source_pixbuf(&pb, 0.0, 0.0);
                        let _ = cr.paint();
                    }
                    None => {
                        fonte(cr, "#1C1D23", 1.0);
                        let _ = cr.paint();
                    }
                }
            });
            col.append(&mini);
            let l = rotulo(&nome, "cfg-estilo-nome");
            l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            l.set_max_width_chars(1);
            col.append(&l);
            b.set_child(Some(&col));
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| if let Some(a) = w.upgrade() { let c = caminho.clone(); a.mudar(|t| t.papel = c) });
            grade.insert(&b, -1);
        }
        self.papeis.append(&grade);
        let escolher = gtk::Button::with_label(tr("Choose a picture…"));
        escolher.add_css_class("cfg-botao");
        escolher.set_halign(Align::Start);
        let w = Rc::downgrade(self);
        escolher.connect_clicked(move |_| {
            let Some(a) = w.upgrade() else { return };
            let filtros = vec![crate::arquivos::seletor::Filtro::de(tr("Pictures"), &["image/*"])];
            let pasta = glib::user_special_dir(glib::UserDirectory::Pictures);
            let w = Rc::downgrade(&a);
            crate::arquivos::seletor::abrir_arquivo(tr("Settings"), tr("Choose a wallpaper"), None, filtros, pasta, move |p| {
                if let Some(a) = w.upgrade() {
                    a.usar_papel(&p.to_string_lossy());
                }
            });
        });
        self.papeis.append(&escolher);
    }

    /// A picture of the person's as the wallpaper (it joins the recent ones).
    pub fn usar_papel(self: &Rc<Self>, p: &str) {
        let p = p.to_string();
        self.mudar(|t| {
            t.papeis.retain(|x| *x != p);
            t.papeis.insert(0, p.clone());
            t.papeis.truncate(5);
            t.papel = Some(p);
        });
        self.preencher();
    }
}
