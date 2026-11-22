//! Barra superior, dock e área de trabalho: superfícies layer-shell ligadas ao estado (niri + kalangod).

use crate::apps::{self, App};
use crate::servicos::niri_acao;
use crate::Shell;
use gtk::prelude::*;
use kalango_i18n::{tr, trf};
use gtk::{gdk, gio, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_movimento::{self as mv, Animavel, Canal};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const BARRA_ALTURA: i32 = 32;

pub fn caixa(o: Orientation, esp: i32, classe: &str) -> gtk::Box {
    let b = gtk::Box::new(o, esp);
    if !classe.is_empty() {
        b.add_css_class(classe);
    }
    b
}

/// Hides `b` while it has no children (GTK's CSS has no `:empty`): a footer that is sometimes empty keeps no padding.
pub fn hide_when_empty(b: &gtk::Box) {
    b.set_visible(b.first_child().is_some());
    let w = b.downgrade();
    let children = b.observe_children();
    children.connect_items_changed(move |m, _, _, _| {
        if let Some(b) = w.upgrade() {
            b.set_visible(m.n_items() > 0);
        }
    });
    // (the box keeps the list alive: dropped here, it stopped telling, and a footer hidden while empty never came
    // back — the runs' "Decidir agora" and "Repetir" were never shown)
    unsafe {
        b.set_data("kalango-filhos-observados", children);
    }
}

pub fn rotulo(t: &str, classe: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(t));
    if !classe.is_empty() {
        l.add_css_class(classe);
    }
    l
}

/// Estrela de quatro pontas do KalangoOS (marca dos agentes): the design's icon/spark — `tamanho` is the star itself
/// (13 in the designs), filled and outlined with 1.5 px; the image is one pixel larger (the outline's half on each
/// side, so it sits where the design's 13 px box is, within half a pixel).
pub fn estrela(tamanho: i32, cor: &str) -> gtk::Image {
    let img = gtk::Image::new();
    pintar_estrela(&img, tamanho, cor);
    let c = cor.to_string();
    crate::tema::registrar(&img, move |w| {
        if let Some(i) = w.downcast_ref::<gtk::Image>() {
            pintar_estrela(i, tamanho, &c);
        }
    });
    let lado = tamanho + 1;
    img.set_pixel_size(lado);
    // its own size, centred (an image stretches a picture to all the room it is given, as inside a button)
    img.set_size_request(lado, lado);
    img.set_halign(gtk::Align::Center);
    img.set_valign(gtk::Align::Center);
    img
}

fn pintar_estrela(img: &gtk::Image, tamanho: i32, cor: &str) {
    let cor = crate::tema::cor(cor);
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="14.5" height="14.5" viewBox="-0.75 -0.75 14.5 14.5"><path d="M6.5 0C7.1 3.6 9.267 5.767 13 6.5C9.267 7.233 7.1 9.4 6.5 13C5.9 9.4 3.733 7.233 0 6.5C3.733 5.767 5.9 3.6 6.5 0Z" fill="{cor}" stroke="{cor}" stroke-width="1.5" stroke-linejoin="round"/></svg>"#);
    let lado = tamanho + 1;
    // drawn at the size it shows (twice, for sharp screens)
    let fluxo = gtk::gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(svg.into_bytes()));
    let tex = gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale(&fluxo, lado * 2, lado * 2, true, None::<&gtk::gio::Cancellable>).ok().map(|pb| gdk::Texture::for_pixbuf(&pb));
    img.set_paintable(tex.as_ref());
}

pub fn camada(app: &gtk::Application, nome: &str, layer: Layer, bordas: &[Edge]) -> gtk::ApplicationWindow {
    let w = gtk::ApplicationWindow::builder().application(app).build();
    w.init_layer_shell();
    w.set_namespace(Some(nome));
    w.set_layer(layer);
    for e in bordas {
        w.set_anchor(*e, true);
    }
    w.set_keyboard_mode(KeyboardMode::None);
    w
}

thread_local! {
    /// the popovers of layer surfaces open now (see `keyboard_while_open`)
    static LAYER_POPOVERS: RefCell<Vec<glib::WeakRef<gtk::Popover>>> = const { RefCell::new(Vec::new()) };
}

/// A popover of a layer surface that takes no keyboard (the dock's menus, the Shelf): while it is open the surface
/// takes the keyboard, so Escape and the arrows reach it — without it such a popover could only be closed with the
/// mouse. The keyboard going to another surface closes it, as a click outside it does.
pub fn keyboard_while_open(pop: &gtk::Popover) {
    let held: Rc<RefCell<Option<(gtk::Window, glib::SignalHandlerId)>>> = Rc::default();
    let h = held.clone();
    pop.connect_map(move |p| {
        let Some(win) = p.root().and_downcast::<gtk::Window>() else { return };
        if !win.is_layer_window() || !matches!(win.keyboard_mode(), KeyboardMode::None) {
            return;
        }
        win.set_keyboard_mode(KeyboardMode::Exclusive);
        LAYER_POPOVERS.with(|l| l.borrow_mut().push(p.downgrade()));
        // (the first change is the keyboard arriving; losing it later closes the popover)
        let weak = p.downgrade();
        let id = win.connect_is_active_notify(move |w| {
            if !w.is_active() {
                if let Some(p) = weak.upgrade() {
                    p.popdown();
                }
            }
        });
        if let Some((old, old_id)) = h.borrow_mut().replace((win, id)) {
            old.disconnect(old_id);
        }
    });
    pop.connect_unmap(move |p| {
        LAYER_POPOVERS.with(|l| l.borrow_mut().retain(|x| x.upgrade().is_some_and(|x| &x != p)));
        if let Some((win, id)) = held.borrow_mut().take() {
            win.disconnect(id);
            win.set_keyboard_mode(KeyboardMode::None);
        }
    });
}

/// A panel of the shell takes the keyboard (Super+V's, the search, the app grid…). A popover of the dock left open
/// goes first: open, it would keep the keyboard on the dock (the compositor leaves the focus with a popup's grab) and
/// GTK would hand it every key of the shell.
pub fn take_keyboard(window: &impl IsA<gtk::Window>) {
    let window: &gtk::Window = window.upcast_ref();
    let open: Vec<gtk::Popover> = LAYER_POPOVERS.with(|l| l.borrow().iter().filter_map(|p| p.upgrade()).collect());
    for p in open {
        if p.root().as_ref().map(|r| r.upcast_ref::<gtk::Widget>()) != Some(window.upcast_ref()) {
            p.popdown();
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug superficies: menu da camada fechado (outro painel pegou o teclado)");
            }
        }
    }
    window.set_keyboard_mode(KeyboardMode::Exclusive);
}

/// A rectangle a full-screen overlay leaves out: x, y, width, height and corner radius (logical pixels).
#[derive(Clone, Copy, Debug, Default)]
pub struct Furo {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub r: f64,
}

/// The top bar and the dock stay above the full-screen overlays (the app grid, the overview): in the design the
/// scrim dims only what is behind them, and they keep working. Their rectangles on an output of this size.
pub fn furos(shell: &Shell, largura: i32, altura: i32) -> Vec<Furo> {
    let mut v = vec![Furo { x: 0.0, y: 0.0, w: f64::from(largura), h: f64::from(BARRA_ALTURA), r: 0.0 }];
    if let Some(d) = shell.dock.borrow().as_ref().filter(|d| d.is_visible() && d.width() > 0) {
        let (w, h) = (f64::from(d.width()), f64::from(d.height()));
        v.push(match estilo_atual().estilo.as_str() {
            "fixa" => Furo { x: 0.0, y: f64::from(altura) - h, w, h, r: 0.0 },
            "lateral" => Furo { x: 8.0, y: (f64::from(altura) - h) / 2.0, w, h, r: 8.0 },
            _ => Furo { x: (f64::from(largura) - w) / 2.0, y: f64::from(altura) - 8.0 - h, w, h, r: 8.0 },
        });
    }
    v
}

thread_local! {
    /// the wallpaper at the size it was last drawn (decoding the 4K file takes a moment: once per size)
    static PAPEL: RefCell<Option<(i32, i32, std::path::PathBuf, gtk::gdk_pixbuf::Pixbuf)>> = const { RefCell::new(None) };
}

/// The session's wallpaper covering `w` × `h` (the version for that proportion, cut in the middle like swaybg's
/// "fill").
pub fn papel_de_parede(w: i32, h: i32) -> Option<gtk::gdk_pixbuf::Pixbuf> {
    let caminho = crate::papel::para(w, h)?;
    if let Some(pb) = PAPEL.with(|p| p.borrow().as_ref().filter(|(pw, ph, c, _)| *pw == w && *ph == h && *c == caminho).map(|(_, _, _, pb)| pb.clone())) {
        return Some(pb);
    }
    let (iw, ih) = gtk::gdk_pixbuf::Pixbuf::file_info(&caminho).map(|(_, w, h)| (w, h))?;
    let k = (f64::from(w) / f64::from(iw)).max(f64::from(h) / f64::from(ih));
    let (sw, sh) = ((f64::from(iw) * k).ceil() as i32, (f64::from(ih) * k).ceil() as i32);
    let pb = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(&caminho, sw, sh, false).ok()?;
    let pb = pb.new_subpixbuf((sw - w).max(0) / 2, (sh - h).max(0) / 2, w.min(sw), h.min(sh));
    PAPEL.with(|p| *p.borrow_mut() = Some((w, h, caminho.clone(), pb.clone())));
    Some(pb)
}

/// The overlays' background: the wallpaper (what is on screen stays behind: the windows become the overlay's
/// pictures, as in the design) and the scrim (#0B0C10 at `alpha`), with the holes cut out.
pub fn veu(alpha: f64, furos: Rc<RefCell<Vec<Furo>>>) -> gtk::DrawingArea {
    veu_de("#0B0C10", alpha, furos)
}

/// A scrim of this colour (the dark sheet's: the theme shows it its own way — the background is light in the light
/// theme) over the wallpaper.
pub fn veu_de(cor: &'static str, alpha: f64, furos: Rc<RefCell<Vec<Furo>>>) -> gtk::DrawingArea {
    use gtk::gdk::prelude::GdkCairoContextExt;
    let a = gtk::DrawingArea::new();
    a.set_hexpand(true);
    a.set_vexpand(true);
    a.set_draw_func(move |_, cr, w, h| {
        if let Some(pb) = papel_de_parede(w, h) {
            cr.set_source_pixbuf(&pb, 0.0, 0.0);
            let _ = cr.paint();
        }
        let (r, g, b) = crate::tema::rgb(cor);
        cr.set_source_rgba(r, g, b, alpha);
        let _ = cr.paint();
        cr.set_operator(gtk::cairo::Operator::Clear);
        for f in furos.borrow().iter() {
            let r = f.r.min(f.w / 2.0).min(f.h / 2.0);
            cr.new_sub_path();
            cr.arc(f.x + f.w - r, f.y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
            cr.arc(f.x + f.w - r, f.y + f.h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
            cr.arc(f.x + r, f.y + f.h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
            cr.arc(f.x + r, f.y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
            cr.close_path();
            let _ = cr.fill();
        }
    });
    a
}

/// Clicks in the holes go through to what is there (the bar, the dock).
pub fn entrada_sem_furos(janela: &gtk::ApplicationWindow, furos: &[Furo]) {
    let Some(s) = janela.surface() else { return };
    // before the first size, the whole surface takes input (an empty region would let every click through)
    let (w, h) = if s.width() > 0 { (s.width(), s.height()) } else { (1 << 15, 1 << 15) };
    let r = gtk::cairo::Region::create_rectangle(&gtk::cairo::RectangleInt::new(0, 0, w, h));
    for f in furos {
        let _ = r.subtract_rectangle(&gtk::cairo::RectangleInt::new(f.x.floor() as i32, f.y.floor() as i32, f.w.ceil() as i32 + 1, f.h.ceil() as i32 + 1));
    }
    s.set_input_region(&r);
}

// ------------------------------------------------------------------------------------------------- barra superior

pub fn barra(app: &gtk::Application, shell: &Rc<Shell>) -> gtk::ApplicationWindow {
    let w = camada(app, "kalango-barra-superior", Layer::Top, &[Edge::Top, Edge::Left, Edge::Right]);
    w.set_exclusive_zone(BARRA_ALTURA);
    w.set_default_size(-1, BARRA_ALTURA);
    let cb = gtk::CenterBox::new();
    cb.add_css_class("barra");

    // esquerda: espaços de trabalho (os da saída focada). A pílula do espaço ativo é uma peça só que desliza
    // até o espaço novo com a mola lenta (percurso longo), por cima dos pontos.
    let espacos = caixa(Orientation::Horizontal, 0, "");
    espacos.set_valign(Align::Center);
    let pilula = caixa(Orientation::Horizontal, 0, "espaco-ativo");
    pilula.set_can_target(false);
    let pilula_a = Animavel::new(&pilula);
    pilula_a.set_halign(Align::Start);
    pilula_a.set_valign(Align::Center);
    pilula_a.set_can_target(false);
    let sobre = gtk::Overlay::new();
    sobre.set_child(Some(&espacos));
    sobre.add_overlay(&pilula_a);
    let moldura = caixa(Orientation::Horizontal, 0, "espacos");
    moldura.set_valign(Align::Center);
    moldura.append(&sobre);
    cb.set_start_widget(Some(&moldura));
    let primeira_vez = std::rc::Rc::new(std::cell::Cell::new(true));
    let conjunto: Rc<RefCell<Vec<u64>>> = Rc::default();

    // centro: a ilha (data e hora)
    let ilha = gtk::Button::new();
    ilha.add_css_class("ilha");
    ilha.set_valign(Align::Center);
    let ib = caixa(Orientation::Horizontal, 8, "");
    // media playing (MPRIS): mini cover, title and artist before the time, as in the design
    let media_box = caixa(Orientation::Horizontal, 6, "");
    let media_title = rotulo("", "ilha-titulo");
    let media_artist = rotulo("", "data");
    media_title.set_max_width_chars(24);
    media_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    media_artist.set_max_width_chars(18);
    media_artist.set_ellipsize(gtk::pango::EllipsizeMode::End);
    // (the playing track's cover in miniature: it slides when the track changes, as in the Music app and the widget)
    let media_cover = crate::midia::CapaDeslizante::new(18);
    media_cover.widget().add_css_class("ilha-capa");
    media_cover.widget().set_valign(Align::Center);
    media_box.append(media_cover.widget());
    media_box.append(&media_title);
    media_box.append(&media_artist);
    media_box.set_visible(false);
    let data = rotulo("", "data");
    let hora = rotulo("", "hora");
    ib.append(&media_box);
    ib.append(&data);
    ib.append(&hora);
    let island_anim = Animavel::new(&ib);
    ilha.set_child(Some(&island_anim));
    {
        // a click expands it into what is happening now (or opens the notices); resting on it, too
        crate::island_card::attach(&ilha, shell);
        let (mb, mt, ma, dt, anim, mc) = (media_box.clone(), media_title.clone(), media_artist.clone(), data.clone(), island_anim.clone(), media_cover.clone());
        let last_title = Rc::new(RefCell::new(String::new()));
        shell.midia.ouvir(move |m| {
            match m.filter(|m| m.tocando) {
                Some(m) => {
                    mt.set_text(&m.titulo);
                    ma.set_text(&m.artista);
                    mc.mostrar(&m);
                    mb.set_visible(true);
                    dt.set_visible(false);
                    // a new track: the island changes state with the elastic spring (feedback with life)
                    if *last_title.borrow() != m.titulo {
                        *last_title.borrow_mut() = m.titulo.clone();
                        anim.definir(Canal::Escala, 0.92);
                        anim.ir(Canal::Escala, 1.0, mv::ELASTICA);
                    }
                }
                None => {
                    mb.set_visible(false);
                    dt.set_visible(true);
                    last_title.borrow_mut().clear();
                }
            }
        });
    }
    // the island, or an agent's request for approval (its own buttons: see pedido_aviso)
    let centro = gtk::Stack::new();
    centro.set_transition_type(gtk::StackTransitionType::Crossfade);
    centro.set_transition_duration(180);
    centro.set_hhomogeneous(false);
    centro.set_interpolate_size(true);
    centro.add_named(&ilha, Some("ilha"));
    if let Some(aviso) = shell.aviso_pedido() {
        let (pilula, texto) = crate::pedido_aviso::pilula_da_ilha(&aviso);
        centro.add_named(&pilula, Some("pedido"));
        let (cw, sw) = (centro.downgrade(), Rc::downgrade(shell));
        aviso.set_ilha(move |t| {
            if let Some(c) = cw.upgrade() {
                match t {
                    Some(t) => {
                        texto.set_text(&t);
                        c.set_visible_child_name("pedido");
                    }
                    // answered: back to the agent's task if it still works, else the clock
                    None => {
                        let trabalhando = sw.upgrade().and_then(|s| s.usando()).is_some_and(|u| u.visivel());
                        if trabalhando { c.set_visible_child_name("tarefa") } else { crate::focus_session::idle_island(&c) }
                    }
                }
            }
        });
    }
    // the island while a long copy runs: its bar; a click opens the card with the speed, the time left, pause, cancel
    if let Some(ops) = shell.operacoes() {
        let (botao, definir) = crate::operacao::ilha_da_operacao();
        centro.add_named(&botao, Some("operacao"));
        let ow = Rc::downgrade(&ops);
        botao.connect_clicked(move |_| {
            if let Some(o) = ow.upgrade() {
                o.expandir();
            }
        });
        let (cw, bw) = (centro.downgrade(), botao.downgrade());
        ops.set_ilha(move |f| {
            let Some(c) = cw.upgrade() else { return };
            // (Settings › Notifications › "Mostrar na ilha" off: the clock stays)
            let f = f.filter(|_| crate::notification_policy::read().in_island);
            // an approval or the agent's task keep the island: they need the person more
            if matches!(c.visible_child_name().as_deref(), Some("pedido") | Some("tarefa")) {
                return;
            }
            match f {
                Some(f) => {
                    definir(f);
                    if c.visible_child_name().as_deref() != Some("operacao") {
                        c.set_visible_child_name("operacao");
                        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                            eprintln!("debug operacao: na ilha");
                            if let Some(b) = bw.upgrade() {
                                crate::central::posicao_debug("ilha operacao", b.upcast_ref());
                            }
                        }
                    }
                }
                None => crate::focus_session::idle_island(&c),
            }
        });
    }
    // the island while the agent works on a task with a known size (Files organising a folder): its progress
    if let Some(u) = shell.usando() {
        let (tarefa, definir) = crate::usando::ilha_da_tarefa();
        centro.add_named(&tarefa, Some("tarefa"));
        let cw = centro.downgrade();
        u.set_ilha(move |t| {
            let Some(c) = cw.upgrade() else { return };
            let t = t.filter(|_| crate::notification_policy::read().in_island);
            // an approval waiting keeps the island: it needs the person more
            if c.visible_child_name().as_deref() == Some("pedido") {
                return;
            }
            match t {
                Some((quem, feitos, total)) => {
                    definir(&quem, feitos, total);
                    c.set_visible_child_name("tarefa");
                }
                None => crate::focus_session::idle_island(&c),
            }
        });
    }
    // a focus session: the island is its timer; a screen recording: its time and stop
    crate::focus_session::island(&centro);
    crate::screenshot_tool::island(&centro);
    centro.set_visible_child_name("ilha");
    cb.set_center_widget(Some(&centro));
    // the time as the person chose (Settings › System › Date: 12 or 24 hours, seconds), read again every 10 s
    let formato: std::rc::Rc<std::cell::Cell<(&'static str, u32)>> = std::rc::Rc::new(std::cell::Cell::new(("%H:%M", 0)));
    let atualizar_hora = move || {
        let agora = glib::DateTime::now_local().unwrap();
        let (mut f, n) = formato.get();
        if n % 10 == 0 {
            let a = crate::configuracoes::ajustes();
            f = match (a["relogio_12h"].as_bool().unwrap_or(false), a["relogio_segundos"].as_bool().unwrap_or(false)) {
                (false, false) => "%H:%M",
                (false, true) => "%H:%M:%S",
                (true, false) => "%I:%M %p",
                (true, true) => "%I:%M:%S %p",
            };
        }
        formato.set((f, n.wrapping_add(1)));
        data.set_text(&agora.format("%a %-d %b").map(|s| s.to_lowercase().replace('.', "")).unwrap_or_default());
        hora.set_text(&agora.format(f).map(|s| s.to_string()).unwrap_or_default());
    };
    atualizar_hora();
    glib::timeout_add_seconds_local(1, move || {
        atualizar_hora();
        glib::ControlFlow::Continue
    });

    // direita: agentes (pedidos do kalangod) e estado do sistema
    let dir = caixa(Orientation::Horizontal, 8, "");
    dir.set_valign(Align::Center);
    // who is watching now (microphone, camera, screen, agents reading): chips no app can hide
    dir.append(&crate::privacy_now::chips());
    let agentes = gtk::Button::new();
    agentes.add_css_class("pilula");
    agentes.add_css_class("agentes");
    let ab = caixa(Orientation::Horizontal, 5, "");
    ab.append(&estrela(14, "#A5A1E2"));
    let atxt = rotulo(tr("agents"), "");
    ab.append(&atxt);
    let pontos = caixa(Orientation::Horizontal, 0, "");
    pontos.set_valign(Align::Center);
    ab.append(&pontos);
    let agentes_a = Animavel::new(&ab);
    agentes.set_child(Some(&agentes_a));
    {
        let s = shell.clone();
        agentes.connect_clicked(move |_| s.right_bar().alternar(crate::lateral::Modo::Agente));
    }
    dir.append(&agentes);
    let ultimo_pendente: Rc<std::cell::Cell<Option<usize>>> = Rc::default();

    // status (design "Status": line icons of the design, 12 px apart, on the #22232B pill)
    let estado_box = caixa(Orientation::Horizontal, 12, "pilula");
    estado_box.add_css_class("estado");
    use crate::conversa::{set_icon, ui_icon};
    const ON: &str = "#E2E2E6";
    const OFF: &str = "#7A7B88";
    let net_icon = ui_icon("wifi", ON, 16);
    let bt_icon = ui_icon("bluetooth", ON, 16);
    let vol_icon = ui_icon("volume", ON, 16);
    for (icon, tip) in [(&net_icon, tr("Network")), (&bt_icon, tr("Bluetooth")), (&vol_icon, tr("Sound"))] {
        let b = gtk::Button::new();
        b.set_child(Some(icon));
        b.set_tooltip_text(Some(tip));
        let s = shell.clone();
        b.connect_clicked(move |_| s.right_bar().alternar(crate::lateral::Modo::Controles));
        estado_box.append(&b);
    }
    {
        let (ni, bi, vi) = (net_icon.clone(), bt_icon.clone(), vol_icon.clone());
        shell.on_system_state(move |st| {
            let (net, color) = if st.rede.aviao {
                ("aviao", ON)
            } else if st.rede.conectada.is_some() {
                (if st.rede.sinal <= 40 { "wifi-fraco" } else { "wifi" }, ON)
            } else if st.rede.cabo {
                ("cabo", ON)
            } else if st.rede.wifi_ligado {
                ("wifi-sem", ON)
            } else {
                ("wifi-sem", OFF)
            };
            set_icon(&ni, net, color);
            ni.set_tooltip_text(Some(&st.rede.conectada.clone().unwrap_or_else(|| if st.rede.cabo { tr("Wired network").to_string() } else { tr("No wireless network").to_string() })));
            bi.set_visible(st.bt.existe);
            if st.bt.ligado { set_icon(&bi, "bluetooth", ON) } else { set_icon(&bi, "bluetooth-off", OFF) }
            let vol = if st.som.mudo { "volume-mudo" } else if st.som.nivel > 0.4 { "volume" } else { "volume-baixo" };
            set_icon(&vi, vol, ON);
            vi.set_tooltip_text(Some(&trf("Volume {pct}%", &[("pct", format!("{:.0}", st.som.nivel * 100.0))])));
        });
    }
    let energia = gtk::Button::new();
    energia.set_child(Some(&ui_icon("energia", ON, 16)));
    energia.set_tooltip_text(Some(tr("Power")));
    {
        let s = shell.clone();
        energia.connect_clicked(move |b| {
            // o menu nasce do ícone: guarda o centro dele (coordenadas da tela = da barra, ancorada no canto)
            if let Some(r) = b.root().and_then(|r| b.compute_bounds(&r)) {
                s.energia().origem.set(((r.x() + r.width() / 2.0) as f64, (r.y() + r.height() / 2.0) as f64));
            }
            s.energia().alternar()
        });
    }
    estado_box.append(&energia);
    dir.append(&estado_box);
    cb.set_end_widget(Some(&dir));
    w.set_child(Some(&cb));

    let est = shell.estado.clone();
    let render = {
        let est = est.clone();
        move || {
            // espaços: reconstrói os pontos só quando o conjunto muda; a pílula desliza até o ativo
            let es = est.espacos.borrow();
            let saida = es.iter().find(|e| e.focado).or(es.first()).map(|e| e.saida.clone()).unwrap_or_default();
            let meus: Vec<_> = es.iter().filter(|e| e.saida == saida).cloned().collect();
            let ids: Vec<u64> = meus.iter().map(|e| e.id).collect();
            if *conjunto.borrow() != ids {
                while let Some(c) = espacos.first_child() {
                    espacos.remove(&c);
                }
                for e in &meus {
                    let b = gtk::Button::new();
                    b.add_css_class("espaco");
                    b.set_valign(Align::Center);
                    b.set_tooltip_text(Some(&trf("Workspace {n}", &[("n", e.idx.to_string())])));
                    let id = e.id;
                    b.connect_clicked(move |_| {
                        let _ = niri_acao(json!({"FocusWorkspace": {"reference": {"Id": id}}}));
                    });
                    espacos.append(&b);
                }
                *conjunto.borrow_mut() = ids;
            }
            if let Some(pos) = meus.iter().position(|e| e.ativo) {
                // posição do ponto ativo: cada ponto ocupa 6 px + 2 px de margem de cada lado
                let x = pos as f64 * 10.0 + 2.0 + 3.0 - 9.0;
                if primeira_vez.replace(false) {
                    pilula_a.definir(Canal::X, x);
                } else {
                    pilula_a.ir(Canal::X, x, mv::LENTA);
                }
            }
            moldura.set_visible(!es.is_empty());
            drop(es);
            // agentes
            while let Some(c) = pontos.first_child() {
                pontos.remove(&c);
            }
            agentes.remove_css_class("pedido");
            agentes.remove_css_class("desligado");
            agentes.remove_css_class("trabalhando");
            let pend = est.pendentes.get();
            let antes = ultimo_pendente.replace(pend).unwrap_or(0);
            match pend {
                None => {
                    atxt.set_text(tr("agents"));
                    agentes.add_css_class("desligado");
                    agentes.set_tooltip_text(Some(tr("kalangod is not responding")));
                }
                Some(0) if est.agent_working.get() => {
                    // the agent is working: three live dots (design "2 agentes •••")
                    atxt.set_text(tr("working"));
                    agentes.add_css_class("trabalhando");
                    for i in 0..3 {
                        let p = caixa(Orientation::Horizontal, 0, "ponto");
                        p.add_css_class("vivo");
                        p.add_css_class(&format!("onda{i}"));
                        pontos.append(&p);
                    }
                    agentes.set_tooltip_text(Some(tr("The agent is working on your request")));
                }
                Some(0) => {
                    atxt.set_text(tr("agents"));
                    let p = caixa(Orientation::Horizontal, 0, "ponto");
                    p.add_css_class("vivo");
                    pontos.append(&p);
                    agentes.set_tooltip_text(Some(tr("No requests waiting for you")));
                }
                Some(n) => {
                    // pedido novo: a pílula pulsa com a mola elástica (feedback com vida)
                    if antes < n {
                        agentes_a.definir(Canal::Escala, 0.86);
                        agentes_a.ir(Canal::Escala, 1.0, mv::ELASTICA);
                    }
                    atxt.set_text(&if n == 1 { tr("1 request").to_string() } else { trf("{n} requests", &[("n", n.to_string())]) });
                    agentes.add_css_class("pedido");
                    for _ in 0..n.min(3) {
                        let p = caixa(Orientation::Horizontal, 0, "ponto");
                        p.add_css_class("pedido");
                        pontos.append(&p);
                    }
                    agentes.set_tooltip_text(Some(tr("Agent requests waiting for your approval")));
                }
            }
        }
    };
    render();
    est.ouvir(render);
    w.present();
    w
}

// ------------------------------------------------------------------------------------------------- dock

struct ItemDock {
    botao: gtk::Button,
    icone: Animavel,
    ind: gtk::Box,
}

/// Abre o app (ou foca a janela usada por último — e a traz de volta se estava minimizada) com o quique da mola
/// elástica no ícone. O app que já está em foco é minimizado, como numa barra de tarefas.
fn tocar_item(shell: &Rc<Shell>, a: &App, icone: &Animavel) {
    let est = &shell.estado;
    let janela = est.janelas.borrow().values().filter(|j| apps::da_janela(&j.app_id, &shell.todos).map(|x| x.id == a.id).unwrap_or(false))
        .max_by_key(|j| j.foco_em).cloned();
    match janela {
        Some(j) if j.focada && !j.minimizada => {
            let _ = niri_acao(json!({"MinimizeWindow": {"id": j.id}}));
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug dock: minimizou {} ({})", j.app_id, j.id);
            }
            icone.definir(Canal::Escala, 0.92);
            icone.ir(Canal::Escala, 1.0, mv::RAPIDA);
        }
        Some(j) => {
            let _ = niri_acao(json!({"FocusWindow": {"id": j.id}}));
            // resposta direta: um toque curto (rápida)
            icone.definir(Canal::Escala, 0.92);
            icone.ir(Canal::Escala, 1.0, mv::RAPIDA);
        }
        None => {
            if let Err(e) = apps::abrir(a) {
                eprintln!("kalango-shell: abrir {}: {e}", a.nome);
                return;
            }
            // app abrindo: dois quiques com a mola elástica
            let i = icone.clone();
            icone.ir(Canal::Y, -12.0, mv::ELASTICA);
            for (ms, y) in [(180u64, 0.0), (420, -7.0), (600, 0.0)] {
                let i = i.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
                    i.ir(Canal::Y, y, mv::ELASTICA);
                });
            }
        }
    }
    shell.comando().fechar();
}


/// Which space each app opens on ("Abrir sempre no espaço"): ~/.config/kalango/espacos-dos-apps.json, {app: index}.
fn espacos_dos_apps() -> serde_json::Map<String, Value> {
    std::fs::read(glib::user_config_dir().join("kalango/espacos-dos-apps.json")).ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

fn salvar_espaco_do_app(app: &str, idx: Option<u64>) {
    let mut m = espacos_dos_apps();
    match idx {
        Some(i) => m.insert(app.to_string(), json!(i)),
        None => m.remove(app),
    };
    let d = glib::user_config_dir().join("kalango");
    let _ = std::fs::create_dir_all(&d);
    let _ = std::fs::write(d.join("espacos-dos-apps.json"), serde_json::to_string_pretty(&Value::Object(m)).unwrap_or_default());
}

/// The window on the space of index `idx` (its output's). Checked 400 ms after each try: it went once it left the
/// space it opened on (that space, left empty, may go away and the indexes shift down: not a failure to retry).
fn mover_para_o_espaco(app: String, id: u64, idx: u64, tentativas: u32, origem: Option<u64>) {
    glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
        let onde = || -> Option<(u64, u64)> {
            let w = crate::servicos::niri_pedido(json!("Windows")).ok()?;
            let ws = w["Windows"].as_array()?.iter().find(|j| j["id"].as_u64() == Some(id))?["workspace_id"].as_u64()?;
            let e = crate::servicos::niri_pedido(json!("Workspaces")).ok()?;
            Some((ws, e["Workspaces"].as_array()?.iter().find(|x| x["id"].as_u64() == Some(ws))?["idx"].as_u64()?))
        };
        let Some((ws, i)) = onde() else { return };
        let foi = match origem {
            None => i == idx,
            Some(o) => ws != o,
        };
        if foi {
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug dock: {app} abriu no espaço {idx}");
            }
            return;
        }
        if tentativas == 0 {
            eprintln!("kalango-shell: {app} não foi para o espaço {idx}");
            return;
        }
        let _ = niri_acao(json!({"MoveWindowToWorkspace": {"window_id": id, "reference": {"Index": idx}, "focus": true}}));
        mover_para_o_espaco(app, id, idx, tentativas - 1, Some(origem.unwrap_or(ws)));
    });
}

/// A new window of an app that has its space goes there (once, when it opens).
pub fn seguir_espacos_dos_apps(shell: &Rc<Shell>) {
    let s = Rc::downgrade(shell);
    shell.estado.ao_abrir_janela(move |id, app_id| {
        let Some(s) = s.upgrade() else { return };
        let Some(a) = apps::da_janela(app_id, &s.todos) else { return };
        if let Some(idx) = espacos_dos_apps().get(&a.id).and_then(Value::as_u64) {
            // (niri tells of the window as it is first committed, before it is placed — a move then may not hold: it
            // is checked and sent again, up to three times)
            mover_para_o_espaco(a.id.clone(), id, idx, 3, None);
        }
    });
}

/// An app's recent files: the system's recent list, the kinds the app opens (the latest 3 that still exist).
fn recentes_do_app(app: &App) -> Vec<gio::File> {
    let tipos: Vec<String> = app.info.supported_types().iter().map(|t| t.to_string()).collect();
    let nome = crate::apps::executable_name(&app.info).unwrap_or_default();
    let mut itens: Vec<gtk::RecentInfo> = gtk::RecentManager::default().items().into_iter()
        .filter(|r| r.is_local() && r.exists())
        // (not the system's temporary files — a print preview's — nor hidden ones)
        .filter(|r| {
            let p = gio::File::for_uri(&r.uri()).path().unwrap_or_default();
            // (a temporary file sits right in the temporary folder; the runtime and cache folders are never the person's)
            let temp = p.parent() == Some(glib::tmp_dir().as_path()) || [glib::user_runtime_dir(), glib::user_cache_dir()].iter().any(|d| p.starts_with(d));
            !temp && !p.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
        })
        .filter(|r| (!nome.is_empty() && r.has_application(&nome)) || tipos.iter().any(|t| *t == r.mime_type().as_str()))
        .collect();
    itens.sort_by_key(|r| std::cmp::Reverse(r.modified().to_unix()));
    itens.into_iter().take(3).map(|r| gio::File::for_uri(&r.uri())).collect()
}

/// A dock icon's menu (design "Menus de contexto", ITEM DA DOCK): a new window, the app's recent files, keep it in
/// the dock, the space it always opens on, and closing all its windows.
fn show_dock_menu(shell: &Rc<Shell>, app: &App, anchor: &gtk::Button) {
    use crate::context_menu::Menu;
    let windows: Vec<u64> = shell.estado.janelas.borrow().values()
        .filter(|j| apps::da_janela(&j.app_id, &shell.todos).map(|x| x.id == app.id).unwrap_or(false)).map(|j| j.id).collect();
    let pinned = shell.fixados.borrow().iter().any(|x| x.id == app.id);
    // (a hidden dock stays out while its menu is open)
    AUTO_HIDE.with(|a| a.menu.set(true));
    let m = Menu::beside(anchor, if estilo_atual().estilo == "lateral" { gtk::PositionType::Right } else { gtk::PositionType::Top });
    let c = m.main.clone();
    {
        let a = app.clone();
        let b = m.item(&c, "mais", tr("New window"), "Ctrl+N", "", move || {
            let _ = apps::abrir(&a);
        });
        crate::central::posicao_debug("dock menu nova janela", b.upcast_ref());
    }
    for f in recentes_do_app(app) {
        let nome = f.basename().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let info = app.info.clone();
        m.item(&c, "doc", &nome, "", "", move || {
            let _ = info.launch(&[f.clone()], gio::AppLaunchContext::NONE);
        });
    }
    Menu::divider(&c);
    {
        let (s, a) = (shell.clone(), app.clone());
        let b = m.item(&c, "estrela-n", tr("Keep in the dock"), if pinned { "✓" } else { "" }, "", move || {
            if pinned {
                s.fixados.borrow_mut().retain(|x| x.id != a.id);
            } else {
                s.fixados.borrow_mut().push(a.clone());
            }
            apps::save_pins(&s.fixados.borrow());
            s.estado.avisar();
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug dock: {} {}", a.id, if pinned { "saiu da dock" } else { "fica na dock" });
            }
        });
        crate::central::posicao_debug("dock menu manter", b.upcast_ref());
    }
    {
        let sub = m.sub(&c, "camadas", tr("Always open on the space"), "espaco");
        let atual = espacos_dos_apps().get(&app.id).and_then(Value::as_u64);
        let mut espacos: Vec<(u64, String)> = shell.estado.espacos.borrow().iter()
            .map(|e| (e.idx, e.nome.clone().unwrap_or_else(|| trf("Space {n}", &[("n", e.idx.to_string())])))).collect();
        espacos.sort();
        espacos.dedup_by_key(|e| e.0);
        let mut opcoes: Vec<(Option<u64>, String)> = vec![(None, tr("Any space").to_string())];
        opcoes.extend(espacos.into_iter().map(|(i, n)| (Some(i), n)));
        for (idx, nome) in opcoes {
            let id = app.id.clone();
            let b = m.item(&sub, if atual == idx { "check" } else { "nenhum" }, &nome, "", "", move || {
                salvar_espaco_do_app(&id, idx);
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug dock: {id} abre sempre no espaço {}", idx.map(|i| i.to_string()).unwrap_or_else(|| "qualquer".into()));
                }
            });
            crate::central::posicao_debug(&format!("dock menu espaco {}", idx.map(|i| i.to_string()).unwrap_or_else(|| "qualquer".into())), b.upcast_ref());
        }
    }
    if !windows.is_empty() {
        Menu::divider(&c);
        m.item(&c, "fechar", tr("Close all windows"), "", "perigo", move || {
            for id in &windows {
                let _ = niri_acao(json!({"CloseWindow": {"id": id}}));
            }
        });
    }
    m.pop.connect_closed(|_| {
        AUTO_HIDE.with(|a| a.menu.set(false));
        if estilo_atual().ocultar && !AUTO_HIDE.with(|a| a.inside.get()) {
            agendar_ocultar(600);
        }
    });
    m.show();
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug dock: menu de {}", app.id);
    }
}

/// The dock's style (designs "Configurações · Aparência" and "· Dock e multitarefa"): floating at the bottom (8 px
/// from the edge), a fixed bar across the bottom, or at the left side; and its icons' size. Kept in
/// ~/.config/kalango/dock.json.
#[derive(Clone, Debug, PartialEq)]
pub struct EstiloDock {
    /// "flutuante", "fixa" or "lateral"
    pub estilo: String,
    pub tamanho: i32,
    /// out of the screen until the pointer touches its edge ("Ocultar automaticamente")
    pub ocultar: bool,
    /// recent and suggested apps after the pinned and open ones ("Mostrar apps recentes e sugeridos")
    pub recentes: bool,
    /// the taskbar's apps: "centro" or "esquerda" (design "Dock fixa · apps à esquerda")
    pub alinhamento: String,
}

/// The dock's style, in a file of its own: it used to share dock.json with the pinned apps (a list), and saving
/// either one wiped the other.
fn estilo_dock_path() -> std::path::PathBuf {
    glib::user_config_dir().join("kalango/dock-estilo.json")
}

pub fn estilo_dock() -> EstiloDock {
    let ler = |p: std::path::PathBuf| std::fs::read(p).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).filter(|v| v.is_object());
    let v = match ler(estilo_dock_path()) {
        Some(v) => v,
        // (the style saved where the pinned apps live, before: it moves to its own file)
        None => match ler(glib::user_config_dir().join("kalango/dock.json")) {
            Some(v) => {
                let p = estilo_dock_path();
                if let Some(d) = p.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                let _ = std::fs::write(&p, v.to_string());
                v
            }
            None => serde_json::Value::Null,
        },
    };
    EstiloDock {
        estilo: v["estilo"].as_str().filter(|s| ["flutuante", "fixa", "lateral"].contains(s)).unwrap_or("flutuante").to_string(),
        tamanho: v["tamanho"].as_i64().map(|t| t.clamp(24, 56) as i32).unwrap_or(40),
        ocultar: v["ocultar"].as_bool().unwrap_or(false),
        recentes: v["recentes"].as_bool().unwrap_or(true),
        alinhamento: v["alinhamento"].as_str().filter(|s| *s == "esquerda").unwrap_or("centro").to_string(),
    }
}

pub fn salvar_estilo_dock(e: &EstiloDock) {
    let p = estilo_dock_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, serde_json::json!({"estilo": e.estilo, "tamanho": e.tamanho, "ocultar": e.ocultar, "recentes": e.recentes, "alinhamento": e.alinhamento}).to_string());
}

thread_local! {
    /// the dock (there is only ever one): its window, its box, its items' size and how to draw it again
    static DOCK: RefCell<Option<(glib::WeakRef<gtk::ApplicationWindow>, glib::WeakRef<gtk::Box>, Rc<dyn Fn()>)>> = const { RefCell::new(None) };
    static DOCK_ESTILO: RefCell<Option<EstiloDock>> = const { RefCell::new(None) };
}

fn estilo_atual() -> EstiloDock {
    DOCK_ESTILO.with(|e| e.borrow().clone()).unwrap_or_else(estilo_dock)
}

fn depurando_dock() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// The edge the dock sits on and its margin there when it shows.
fn borda_da_dock(e: &EstiloDock) -> (Edge, i32) {
    match e.estilo.as_str() {
        "fixa" => (Edge::Bottom, 0),
        "lateral" => (Edge::Left, 8),
        _ => (Edge::Bottom, 8),
    }
}

/// "Ocultar automaticamente": how far the dock is out of the screen (a spring from 0, showing, to 1, out), whether the
/// pointer is on it or its menu is open (it stays), and the timers that move it.
struct AutoHide {
    spring: Cell<mv::Mola>,
    inside: Cell<bool>,
    menu: Cell<bool>,
    timer: RefCell<Option<glib::SourceId>>,
    anim: RefCell<Option<glib::SourceId>>,
}

thread_local! {
    static AUTO_HIDE: AutoHide = AutoHide { spring: Cell::new(mv::Mola::new(0.0)), inside: Cell::new(false), menu: Cell::new(false),
        timer: RefCell::default(), anim: RefCell::default() };
}

fn janela_da_dock() -> Option<gtk::ApplicationWindow> {
    DOCK.with(|x| x.borrow().as_ref().and_then(|(w, _, _)| w.upgrade()))
}

/// Out of the screen (or on its way out).
pub fn dock_oculta() -> bool {
    AUTO_HIDE.with(|a| a.spring.get().alvo >= 0.5)
}

/// The dock's margin on its edge for where the spring is: at 1 the whole dock is past the edge.
fn aplicar_margem_oculta(w: &gtk::ApplicationWindow, e: &EstiloDock) {
    let (borda, m) = borda_da_dock(e);
    let x = AUTO_HIDE.with(|a| a.spring.get().x);
    let grossura = if borda == Edge::Left { w.width() } else { w.height() };
    let grossura = if grossura > 0 { grossura } else { espessura(e) + 4 };
    w.set_margin(borda, m - (f64::from(grossura + m + 2) * x).round() as i32);
}

/// Moves the dock in (0) or out (1): the quick spring coming, the soft one going (at once with reduced motion).
fn mover_dock(alvo: f64) {
    let Some(w) = janela_da_dock() else { return };
    AUTO_HIDE.with(|a| {
        let mut s = a.spring.get();
        if mv::reduzido() {
            s.pular(alvo);
        } else {
            s.ir(alvo, if alvo < 0.5 { mv::RAPIDA } else { mv::SUAVE });
        }
        a.spring.set(s);
        if a.anim.borrow().is_some() {
            return;
        }
        // (a timer, not the frame clock: out of the screen the surface gets no frames)
        let w = w.downgrade();
        let mut antes = std::time::Instant::now();
        let id = glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
            let parar = || {
                AUTO_HIDE.with(|a| a.anim.borrow_mut().take());
                glib::ControlFlow::Break
            };
            let Some(w) = w.upgrade() else { return parar() };
            let agora = std::time::Instant::now();
            let dt = (agora - antes).as_secs_f64().min(0.05);
            antes = agora;
            let parada = AUTO_HIDE.with(|a| {
                let mut s = a.spring.get();
                s.passo(dt);
                a.spring.set(s);
                s.parada()
            });
            aplicar_margem_oculta(&w, &estilo_atual());
            if parada { parar() } else { glib::ControlFlow::Continue }
        });
        *a.anim.borrow_mut() = Some(id);
    });
}

fn cancelar_ocultar() {
    AUTO_HIDE.with(|a| {
        if let Some(t) = a.timer.borrow_mut().take() {
            t.remove();
        }
    });
}

fn agendar_ocultar(ms: u64) {
    cancelar_ocultar();
    let id = glib::timeout_add_local_once(std::time::Duration::from_millis(ms), || {
        AUTO_HIDE.with(|a| a.timer.borrow_mut().take());
        ocultar_dock();
    });
    AUTO_HIDE.with(|a| *a.timer.borrow_mut() = Some(id));
}

/// The dock comes out (the edge under it was touched): it goes back a moment after the pointer leaves it, or if the
/// pointer never comes to it.
pub fn mostrar_dock() {
    cancelar_ocultar();
    if !dock_oculta() {
        return;
    }
    mover_dock(0.0);
    crate::edges::dock_zone(false);
    if depurando_dock() {
        eprintln!("debug dock: à mostra");
    }
    if estilo_atual().ocultar {
        agendar_ocultar(1500);
    }
}

fn ocultar_dock() {
    if !estilo_atual().ocultar || dock_oculta() || AUTO_HIDE.with(|a| a.inside.get() || a.menu.get()) {
        return;
    }
    mover_dock(1.0);
    crate::edges::dock_zone(true);
    if depurando_dock() {
        eprintln!("debug dock: oculta");
    }
}

/// The setting as it is now: turned on, the dock goes out a moment later; off, it is back at once.
fn ajustar_ocultar(w: &gtk::ApplicationWindow, e: &EstiloDock) {
    if !e.ocultar {
        cancelar_ocultar();
        AUTO_HIDE.with(|a| {
            let mut s = a.spring.get();
            s.pular(0.0);
            a.spring.set(s);
        });
    }
    aplicar_margem_oculta(w, e);
}

/// The room the dock keeps at the bottom of the screen (its exclusive zone): what a bottom layer such as the
/// on-screen keyboard is pushed up by.
pub(crate) fn zona_da_dock_embaixo() -> i32 {
    let e = estilo_atual();
    if e.ocultar || e.estilo == "lateral" {
        0
    } else if e.estilo == "fixa" {
        espessura(&e)
    } else {
        espessura(&e) + 8
    }
}

/// How thick the dock is: the icon, its indicator (4) and the padding and border (10).
fn espessura(e: &EstiloDock) -> i32 {
    // (the taskbar is 52 px with 40 px icons, as in the design)
    if e.estilo == "fixa" { (e.tamanho + 12).max(52) } else { e.tamanho + 14 }
}

thread_local! {
    /// the taskbar's parts (design "Dock fixa · barra de tarefas"): the bar, its sides, and the apps with the task view
    static BARRA_DE_TAREFAS: RefCell<Option<(gtk::CenterBox, crate::taskbar::Parts, gtk::Box)>> = const { RefCell::new(None) };
}

/// The fixed dock is the taskbar: the top bar steps aside.
pub fn modo_barra_de_tarefas() -> bool {
    estilo_atual().estilo == "fixa"
}

/// The taskbar's sides shown or not, the apps in the middle or on the left, and the top bar.
fn arrumar_barra_de_tarefas(e: &EstiloDock) {
    let fixa = e.estilo == "fixa";
    BARRA_DE_TAREFAS.with(|b| {
        let Some((cb, p, centro)) = b.borrow().as_ref().map(|(cb, p, c)| (cb.clone(), (p.left.clone(), p.right.clone(), p.tasks.clone(), p.divider.clone()), c.clone())) else { return };
        let (left, right, tasks, divider) = p;
        left.set_visible(fixa);
        right.set_visible(fixa);
        tasks.set_visible(fixa);
        let esquerda = fixa && e.alinhamento == "esquerda";
        divider.set_visible(esquerda);
        if fixa && std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            crate::taskbar::log_places(b.borrow().as_ref().map(|x| &x.1).unwrap());
        }
        let no_centro = centro.parent().is_some_and(|x| x == cb.clone().upcast::<gtk::Widget>());
        if esquerda && no_centro {
            cb.set_center_widget(gtk::Widget::NONE);
            left.append(&centro);
        } else if !esquerda && !no_centro {
            left.remove(&centro);
            cb.set_center_widget(Some(&centro));
        }
    });
    if let Some(s) = crate::shell_atual() {
        if let Some(b) = s.barra.borrow().as_ref() {
            let voltou = !fixa && !b.is_visible();
            b.set_visible(!fixa);
            // (the compositor gives each bar its room in the order they appeared: the top bar coming back after the
            // dock would start after a side dock — the dock appears again after it, so the bar spans the screen)
            if voltou {
                if let Some(d) = s.dock.borrow().as_ref().filter(|d| d.is_visible()) {
                    d.set_visible(false);
                    d.set_visible(true);
                }
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug dock: a barra superior voltou antes da dock ({})", e.estilo);
                }
            }
        }
        s.notificacoes().apply_position();
    }
}

/// Puts the dock where its style says: anchors, margin, the room it keeps for itself and its direction.
fn posicionar_dock(w: &gtk::ApplicationWindow, d: &gtk::Box, e: &EstiloDock) {
    for b in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        w.set_anchor(b, false);
        w.set_margin(b, 0);
    }
    for c in ["fixa", "lateral"] {
        d.remove_css_class(c);
    }
    match e.estilo.as_str() {
        "fixa" => {
            for b in [Edge::Bottom, Edge::Left, Edge::Right] {
                w.set_anchor(b, true);
            }
            w.set_exclusive_zone(espessura(e));
            d.set_orientation(Orientation::Horizontal);
            d.add_css_class("fixa");
        }
        "lateral" => {
            w.set_anchor(Edge::Left, true);
            w.set_margin(Edge::Left, 8);
            w.set_exclusive_zone(espessura(e) + 8);
            d.set_orientation(Orientation::Vertical);
            d.add_css_class("lateral");
        }
        _ => {
            w.set_anchor(Edge::Bottom, true);
            w.set_margin(Edge::Bottom, 8);
            // the dock's room (and 8 px of margin): maximized windows stay above it
            w.set_exclusive_zone(espessura(e) + 8);
            d.set_orientation(Orientation::Horizontal);
        }
    }
    if e.ocultar {
        // (the windows keep the whole screen: the dock comes over them)
        w.set_exclusive_zone(0);
    }
    ajustar_ocultar(w, e);
    // the dock's box and the Shelf beside it follow its direction (a side dock: the pile below its icons)
    let o = d.orientation();
    if let Some(c) = d.parent().and_downcast::<gtk::Box>() {
        c.set_orientation(o);
    }
    crate::shelf::follow(o);
    // a fixed bar spans the screen (the window is the bar), its icons in the middle
    if e.estilo == "fixa" {
        w.add_css_class("dock-fixa");
    } else {
        w.remove_css_class("dock-fixa");
    }
    d.set_halign(Align::Center);
}

/// The dock in its style now (after a change in the settings): the same dock, moved and drawn again.
pub fn refazer_dock() {
    let e = estilo_dock();
    DOCK_ESTILO.with(|x| *x.borrow_mut() = Some(e.clone()));
    let Some((w, d, render)) = DOCK.with(|x| x.borrow().as_ref().and_then(|(w, d, r)| Some((w.upgrade()?, d.upgrade()?, r.clone())))) else { return };
    posicionar_dock(&w, &d, &e);
    arrumar_barra_de_tarefas(&e);
    render();
    // the edge that brings a hidden dock back
    crate::edges::apply();
    if e.ocultar && !dock_oculta() && !AUTO_HIDE.with(|a| a.inside.get()) {
        agendar_ocultar(800);
    }
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug dock: {} {} px{}{}", e.estilo, e.tamanho, if e.ocultar { ", oculta" } else { "" }, if e.recentes { ", recentes" } else { "" });
    }
}

pub fn dock(app: &gtk::Application, shell: &Rc<Shell>) -> gtk::ApplicationWindow {
    let w = camada(app, "kalango-dock", Layer::Top, &[Edge::Bottom]);
    let d = caixa(Orientation::Horizontal, 4, "dock");
    let e = estilo_dock();
    DOCK_ESTILO.with(|x| *x.borrow_mut() = Some(e.clone()));
    if e.ocultar {
        // a session starting with the dock hidden
        AUTO_HIDE.with(|a| {
            let mut s = a.spring.get();
            s.pular(1.0);
            a.spring.set(s);
        });
    }
    posicionar_dock(&w, &d, &e);
    // the same window is the taskbar in the fixed style: its sides around the apps (hidden in the other styles)
    let partes = crate::taskbar::build(shell);
    let centro = caixa(Orientation::Horizontal, 4, "");
    centro.set_valign(Align::Center);
    centro.append(&partes.tasks);
    centro.append(&d);
    // the Shelf, beside the dock while something is new (design "Prateleira")
    centro.append(&crate::shelf::widget());
    centro.set_orientation(d.orientation());
    crate::shelf::follow(d.orientation());
    let cb = gtk::CenterBox::new();
    cb.set_start_widget(Some(&partes.left));
    cb.set_center_widget(Some(&centro));
    cb.set_end_widget(Some(&partes.right));
    w.set_child(Some(&cb));
    BARRA_DE_TAREFAS.with(|b| *b.borrow_mut() = Some((cb, partes, centro)));
    // the pointer on the dock keeps a hidden one out; leaving it, it goes back
    let sobre = gtk::EventControllerMotion::new();
    sobre.connect_enter(|_, _, _| {
        AUTO_HIDE.with(|a| a.inside.set(true));
        cancelar_ocultar();
    });
    sobre.connect_leave(|_| {
        AUTO_HIDE.with(|a| a.inside.set(false));
        if estilo_atual().ocultar {
            agendar_ocultar(600);
        }
    });
    w.add_controller(sobre);
    let itens: Rc<RefCell<Vec<(String, ItemDock)>>> = Rc::default();
    // the icons' size they were made at (a new size makes them again)
    let feito_em: Rc<Cell<(i32, bool)>> = Rc::new(Cell::new((0, false)));

    let render = {
        let shell = shell.clone();
        let d = d.clone();
        move || {
            let est = &shell.estado;
            let js = est.janelas.borrow();
            let mut lista: Vec<App> = shell.fixados.borrow().clone();
            for j in js.values() {
                if let Some(a) = apps::da_janela(&j.app_id, &shell.todos) {
                    if !lista.iter().any(|x| x.id == a.id) {
                        lista.push(a.clone());
                    }
                }
            }
            let e = estilo_atual();
            // recent and suggested apps after the pinned and open ones, past a separator (Settings › Appearance ›
            // Dock and multitasking)
            let ja: Vec<String> = lista.iter().map(|a| a.id.clone()).collect();
            let extras: Vec<(App, bool)> = if e.recentes {
                apps::recent_and_suggested(&ja, |id| shell.todos.iter().any(|a| a.id == id), 3).into_iter()
                    .filter_map(|(id, sug)| shell.todos.iter().find(|a| a.id == id).map(|a| (a.clone(), sug))).collect()
            } else {
                Vec::new()
            };
            // só reconstrói quando o conjunto de apps muda; senão atualiza no lugar (animações seguem)
            let mut ids: Vec<String> = ja.clone();
            ids.extend(extras.iter().map(|(a, sug)| format!("+{}{}", a.id, if *sug { "*" } else { "" })));
            let lateral = e.estilo == "lateral";
            let mudou = itens.borrow().iter().map(|(i, _)| i.clone()).collect::<Vec<_>>() != ids || feito_em.get() != (e.tamanho, lateral);
            feito_em.set((e.tamanho, lateral));
            if mudou {
                while let Some(c) = d.first_child() {
                    d.remove(&c);
                }
                let mut novos = Vec::new();
                let entradas: Vec<(&App, String, Option<bool>)> = lista.iter().map(|a| (a, a.id.clone(), None))
                    .chain(extras.iter().zip(ids[ja.len()..].iter()).map(|((a, sug), id)| (a, id.clone(), Some(*sug)))).collect();
                if depurando_dock() && itens.borrow().iter().filter(|(i, _)| i.starts_with('+')).map(|(i, _)| i.clone()).collect::<Vec<_>>() != ids[ja.len()..] {
                    eprintln!("debug dock: recentes e sugeridos: [{}]", ids[ja.len()..].join(", "));
                }
                for (a, id, extra) in entradas {
                    if extra.is_some() && !novos.iter().any(|(i, _): &(String, ItemDock)| i.starts_with('+')) {
                        let sep = caixa(Orientation::Horizontal, 0, "separador");
                        sep.set_valign(Align::Center);
                        d.append(&sep);
                    }
                    let b = gtk::Button::new();
                    b.add_css_class("item");
                    match extra {
                        Some(true) => b.set_tooltip_text(Some(&format!("{} · {}", a.nome, tr("suggested")))),
                        Some(false) => b.set_tooltip_text(Some(&format!("{} · {}", a.nome, tr("opened recently")))),
                        None => b.set_tooltip_text(Some(&a.nome)),
                    }
                    // the indicator under the icon, or beside it on a side dock
                    let col = caixa(if lateral { Orientation::Horizontal } else { Orientation::Vertical }, 0, "");
                    let icone = Animavel::new(&apps::imagem(a, e.tamanho));
                    icone.origem(0.5, 1.0);
                    let ind = caixa(Orientation::Horizontal, 0, "ind");
                    if lateral {
                        ind.set_valign(Align::Center);
                        col.append(&ind);
                        col.append(&icone);
                    } else {
                        ind.set_halign(Align::Center);
                        col.append(&icone);
                        col.append(&ind);
                    }
                    b.set_child(Some(&col));
                    // hover: o ícone sobe 3 px (rápida); sair do hover desce
                    let mov = gtk::EventControllerMotion::new();
                    let (i1, i2) = (icone.clone(), icone.clone());
                    mov.connect_enter(move |_, _, _| {
                        i1.ir(Canal::Y, -3.0, mv::RAPIDA);
                    });
                    mov.connect_leave(move |_| {
                        i2.ir(Canal::Y, 0.0, mv::RAPIDA);
                    });
                    b.add_controller(mov);
                    let (s, app, ic) = (shell.clone(), a.clone(), icone.clone());
                    b.connect_clicked(move |_| {
                        crate::window_preview::hide();
                        tocar_item(&s, &app, &ic)
                    });
                    // the taskbar's window previews
                    crate::window_preview::attach(&b, &shell, &a.id);
                    // right click: context menu
                    let right_click = gtk::GestureClick::new();
                    right_click.set_button(3);
                    // (the button weakly: its own controller holding it strongly kept every dock ever built alive,
                    // with its icons — the dock is built again each time an unpinned app opens or closes)
                    let (s, app, button) = (shell.clone(), a.clone(), b.downgrade());
                    right_click.connect_pressed(move |_, _, _, _| {
                        if let Some(button) = button.upgrade() {
                            show_dock_menu(&s, &app, &button)
                        }
                    });
                    b.add_controller(right_click);
                    d.append(&b);
                    novos.push((id, ItemDock { botao: b, icone, ind }));
                }
                let sep = caixa(Orientation::Horizontal, 0, "separador");
                sep.set_valign(Align::Center);
                // (the taskbar has its own start and grid at the left)
                sep.set_visible(e.estilo != "fixa");
                d.append(&sep);
                let todos = gtk::Button::new();
                todos.set_visible(e.estilo != "fixa");
                todos.add_css_class("todos-apps");
                todos.set_valign(Align::Center);
                todos.set_halign(Align::Center);
                todos.set_size_request(e.tamanho, e.tamanho);
                todos.set_child(Some(&gtk::Image::from_icon_name("view-app-grid-symbolic")));
                todos.set_tooltip_text(Some(tr("Show apps")));
                let s = shell.clone();
                todos.connect_clicked(move |_| s.grade().alternar());
                d.append(&todos);
                // apps novos entram em cascata
                let novos_anim: Vec<Animavel> = novos.iter().filter(|(id, _)| !itens.borrow().iter().any(|(x, _)| x == id)).map(|(_, it)| it.icone.clone()).collect();
                if !itens.borrow().is_empty() {
                    mv::cascata(&novos_anim, 8.0);
                }
                *itens.borrow_mut() = novos;
            }
            for (id, it) in itens.borrow().iter() {
                let janelas: Vec<_> = js.values().filter(|j| apps::da_janela(&j.app_id, &shell.todos).map(|x| &x.id == id).unwrap_or(false)).collect();
                let focado = janelas.iter().any(|j| j.focada);
                for (c, on) in [("focado", focado)] {
                    if on { it.botao.add_css_class(c) } else { it.botao.remove_css_class(c) }
                }
                it.ind.remove_css_class("focado");
                it.ind.remove_css_class("aberto");
                it.ind.remove_css_class("minimizado");
                if focado {
                    it.ind.add_css_class("focado");
                } else if !janelas.is_empty() {
                    it.ind.add_css_class("aberto");
                    // (every window of the app minimized: the dot dims)
                    if janelas.iter().all(|j| j.minimizada) {
                        it.ind.add_css_class("minimizado");
                    }
                }
            }
        }
    };
    let render: Rc<dyn Fn()> = Rc::new(render);
    render();
    {
        let r = render.clone();
        shell.estado.ouvir(move || r());
    }
    DOCK.with(|x| *x.borrow_mut() = Some((w.downgrade(), d.downgrade(), render)));
    w.present();
    w
}

// ------------------------------------------------------------------------------------------------- área de trabalho

/// How long a closed app window waits, hidden, before what it holds of the display is let go
/// (`KALANGO_SOLTAR_JANELA_MS` sets it, for the tests).
fn release_delay() -> std::time::Duration {
    let ms = RELEASE_DELAY_SET.with(|d| d.get()).unwrap_or_else(|| crate::release_policy::delay_ms(std::env::var("KALANGO_SOLTAR_JANELA_MS").ok().as_deref()));
    // (glib counts the delay in 32-bit milliseconds: past that it would wrap round to a short one)
    std::time::Duration::from_millis(ms.min(u64::from(u32::MAX)))
}

thread_local! {
    /// The delay set while the shell runs (`kalango-shell estado prazo MS`, the window tests: a pass with windows kept
    /// hidden as before, one with them let go, in the same session)
    static RELEASE_DELAY_SET: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// For the window tests: the delay of the windows closed from now on.
pub(crate) fn set_release_delay(ms: u64) {
    RELEASE_DELAY_SET.with(|d| d.set(Some(ms)));
}

/// An app of the shell hides its window on close (it opens again as it was, at once); after a while hidden, the window
/// lets go of what it holds of the display — its surface, its renderer with its buffers and textures, its popovers'
/// surfaces — and keeps everything else: every widget and what is in it (the text typed, the page, the scroll, the
/// size), the app and its work. Opened again, GTK realizes it anew (some tens of ms). Kept realized, each app ever
/// opened held tens of MB of renderer until the session ended.
///
/// `ready` answers false to wait a while longer (it is asked again after the delay): something that a surface going
/// away could change (a video playing). A dialog open over the window holds it too (its parent's surface must stay).
pub(crate) fn release_resources_when_hidden(window: &impl IsA<gtk::Window>, ready: impl Fn() -> bool + 'static) {
    release_resources_when_hidden_after(window, None, ready);
}

/// `delay`: none for the shell's (read each time the window is hidden: `estado prazo` may change it)
fn release_resources_when_hidden_after(window: &impl IsA<gtk::Window>, delay: Option<std::time::Duration>, ready: impl Fn() -> bool + 'static) {
    let timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    let ready: Rc<dyn Fn() -> bool> = Rc::new(ready);
    let window: &gtk::Window = window.as_ref();
    if std::env::var_os("KALANGO_MEDIR_JANELAS").is_some() {
        log_first_frame(window);
    }
    window.connect_visible_notify(move |w| {
        if let Some(id) = timer.borrow_mut().take() {
            id.remove();
        }
        if !w.is_visible() {
            arm_unrealize(&w.downgrade(), &timer, &ready, delay.unwrap_or_else(release_delay));
        }
    });
}

/// (the window tests) When the window, mapped, has painted its first frame — and whether it had to be realized anew.
fn log_first_frame(window: &gtk::Window) {
    let fresh = Rc::new(Cell::new(false));
    let f = fresh.clone();
    window.connect_realize(move |_| f.set(true));
    window.connect_map(move |w| {
        let Some(clock) = w.frame_clock() else { return };
        let (id, f, name) = (Rc::new(RefCell::new(None::<glib::SignalHandlerId>)), fresh.clone(), w.widget_name());
        let id2 = id.clone();
        *id.borrow_mut() = Some(clock.connect_after_paint(move |c| {
            if let Some(h) = id2.borrow_mut().take() {
                eprintln!("debug medir: {name} pintada t={} {}", glib::monotonic_time(), if f.replace(false) { "realizada de novo" } else { "já realizada" });
                c.disconnect(h);
            }
        }));
    });
}

/// A dialog of the window's own (transient for it) is on screen.
fn has_open_dialog(win: &gtk::Window) -> bool {
    gtk::Window::list_toplevels().iter().filter_map(|t| t.downcast_ref::<gtk::Window>()).any(|t| t.is_visible() && t.transient_for().as_ref() == Some(win))
}

fn arm_unrealize(w: &glib::WeakRef<gtk::Window>, timer: &Rc<RefCell<Option<glib::SourceId>>>, ready: &Rc<dyn Fn() -> bool>, delay: std::time::Duration) {
    let (w, t, r) = (w.clone(), timer.clone(), ready.clone());
    let id = glib::timeout_add_local_once(delay, move || {
        t.borrow_mut().take();
        let Some(win) = w.upgrade() else { return };
        if win.is_visible() || !win.is_realized() {
            return;
        }
        if !r() || has_open_dialog(&win) {
            arm_unrealize(&w, &t, &r, delay);
            return;
        }
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug janela: «{}» desrealizada depois de fechada ({} superfícies)", win.title().unwrap_or_default(), crate::window_state::realized_natives(win.upcast_ref()));
        }
        // (the widget's unrealize — gtk_widget_unrealize —, not NativeExt's, which is for implementations)
        gtk::prelude::WidgetExt::unrealize(&win);
        // (what it held is free now, but the allocator keeps the pages: given back, the RSS comes down)
        glib::timeout_add_local_once(std::time::Duration::from_millis(500), crate::mem_debug::trim);
    });
    *timer.borrow_mut() = Some(id);
}

/// Runs `f` every `period` while `w` is mapped — a window on screen, a page shown —, as a repeating timer would; while
/// it is not (a closed window, minimized ones stay mapped) the timer is taken out, and it starts again when `w` maps.
/// For the jobs that only work while their widget shows: closed, they woke the shell for nothing. `f` answers false to
/// stop for good.
pub(crate) fn every_while_mapped(w: &impl IsA<gtk::Widget>, period: std::time::Duration, f: impl Fn() -> bool + 'static) {
    let source: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    let f: Rc<dyn Fn() -> bool> = Rc::new(f);
    let done = Rc::new(Cell::new(false));
    let start = {
        let (source, f, done) = (source.clone(), f.clone(), done.clone());
        move || {
            if done.get() || source.borrow().is_some() {
                return;
            }
            let (s, f, d) = (source.clone(), f.clone(), done.clone());
            let id = glib::timeout_add_local(period, move || {
                if f() {
                    return glib::ControlFlow::Continue;
                }
                d.set(true);
                // (ended by Break: taken out without removing it)
                s.borrow_mut().take();
                glib::ControlFlow::Break
            });
            *source.borrow_mut() = Some(id);
        }
    };
    let w: &gtk::Widget = w.as_ref();
    if w.is_mapped() {
        start();
    }
    w.connect_map(move |_| start());
    w.connect_unmap(move |_| {
        if let Some(id) = source.borrow_mut().take() {
            id.remove();
        }
    });
}

/// The size an app's window opens at: its design's, or almost all of a smaller screen (the bars and the dock stay in
/// view) — the screen as niri has it now, not GDK's monitor (which can keep an old size after a scale change: a
/// window opened after the scale went from 125% back to 100% came out at half the size).
pub(crate) fn tamanho_de_janela(largura: i32, altura: i32) -> (i32, i32) {
    let (sw, sh) = tamanho_da_saida()
        .or_else(|| gdk::Display::default().and_then(|d| d.monitors().item(0)).and_downcast::<gdk::Monitor>().map(|m| (m.geometry().width(), m.geometry().height())))
        .unwrap_or((1920, 1080));
    (largura.min(sw - 48), altura.min(sh - 116))
}

/// The output's logical size as niri has it now — GDK's monitor can keep an old size after a scale change. For the
/// session test's position logs (one IPC request each).
pub(crate) fn tamanho_da_saida() -> Option<(i32, i32)> {
    let v = crate::servicos::niri_pedido(serde_json::json!("Outputs")).ok()?;
    let lg = v.get("Outputs")?.as_object()?.values().find_map(|s| s.get("logical").filter(|l| !l.is_null()).cloned())?;
    Some((lg["width"].as_i64()? as i32, lg["height"].as_i64()? as i32))
}

#[cfg(test)]
mod release_tests {
    use super::release_resources_when_hidden_after;
    use gtk::glib;
    use gtk::prelude::*;
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    fn run(ms: u64) {
        let ctx = glib::MainContext::default();
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            ctx.iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Needs a display of its own (it maps a window): KALANGO_TESTE_GTK=1 with DISPLAY on a virtual one —
    /// `Xvfb :119 & KALANGO_TESTE_GTK=1 DISPLAY=:119 GDK_BACKEND=x11 WAYLAND_DISPLAY= cargo test release_tests`, or in
    /// CI `KALANGO_TESTE_GTK=1 GDK_BACKEND=x11 xvfb-run -a cargo test -p kalango-shell release_tests` (one test: GTK
    /// lives on the one thread that started it, gtk4 `rt.rs`: this test and `help_app::tests::closing_the_window_stops_the_listening`
    /// cannot share one process, so one filter per run:
    /// `KALANGO_TESTE_GTK=1 xvfb-run -a cargo test -p kalango-shell --bin kalango-shell -- --test-threads=1 release_tests`).
    #[test]
    fn a_closed_window_lets_go_of_its_surface_and_keeps_its_widgets() {
        if std::env::var_os("KALANGO_TESTE_GTK").is_none() || gtk::init().is_err() {
            eprintln!("(sem KALANGO_TESTE_GTK e uma tela: pulado)");
            return;
        }
        let win = gtk::Window::new();
        let field = gtk::Entry::new();
        field.set_text("o que a pessoa digitou");
        win.set_child(Some(&field));
        let (asked, allow) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(true)));
        let (a, l) = (asked.clone(), allow.clone());
        release_resources_when_hidden_after(&win, Some(Duration::from_millis(150)), move || {
            a.set(a.get() + 1);
            l.get()
        });
        // opened again while still hidden: the timer is cancelled, the window stays realized
        win.set_visible(true);
        run(50);
        win.set_visible(false);
        run(60);
        win.set_visible(true);
        run(300);
        assert_eq!(asked.get(), 0, "shown again before the delay");
        assert!(win.is_realized());
        // busy: asked, kept realized, and asked again later
        allow.set(false);
        win.set_visible(false);
        run(400);
        assert!(asked.get() >= 2, "asked again while busy ({})", asked.get());
        assert!(win.is_realized(), "kept while busy");
        // a dialog of its own open over it: kept too
        allow.set(true);
        let dialog = gtk::Window::builder().transient_for(&win).build();
        dialog.set_visible(true);
        run(300);
        assert!(win.is_realized(), "kept while its dialog is open");
        dialog.set_visible(false);
        // free: unrealized, and everything in it stays
        run(300);
        assert!(!win.is_realized(), "its surface let go once free");
        assert_eq!(field.text(), "o que a pessoa digitou");
        assert_eq!(win.child().as_ref(), Some(field.upcast_ref::<gtk::Widget>()));
        // opened again: realized anew, as it was
        win.set_visible(true);
        run(100);
        assert!(win.is_realized() && field.is_realized());
        assert_eq!(field.text(), "o que a pessoa digitou");
        // (the test's own windows go)
        for w in [win, dialog] {
            w.destroy();
        }
        // (here too: GTK lives on this thread) an icon drawn for one scale is drawn again when the screens' scale changes
        let width = |i: &gtk::Image| i.paintable().and_downcast::<gtk::gdk::Texture>().map(|t| t.width());
        let icon = crate::apps::imagem_kalango("agentes", 48);
        let first = width(&icon).expect("the icon drawn as a texture");
        crate::apps::redraw_icons_at(2.0);
        assert_eq!(width(&icon), Some(crate::apps::icon_pixels(48, 2.0)), "drawn again at twice the pixels (was {first})");
        crate::apps::redraw_icons_at(1.0);
        assert_eq!(width(&icon), Some(48), "and back");
    }
}
