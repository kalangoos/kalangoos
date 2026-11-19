//! KalangoOS's own Music app (design "Música", 58676; app id os.kalango.Musica; `kalango-shell musica [FILE]`):
//! the person's Music folder (music_library.rs), the track playing with its cover, its real waveform (drag it to
//! go anywhere) and the controls; "A seguir"; a mix the local model makes from the library for the moment or for a
//! request ("algo calmo para programar"); favourites and the mixes kept. It plays through GTK's media (GStreamer)
//! and tells the rest of the system through MPRIS, so the island and the desktop's widget show the same cover.
//! In a test profile it plays muted.

use crate::conversa::{set_icon, ui_icon};
use crate::music_library::{self as lib, Mix, Track};
use crate::superficies::{caixa, rotulo};
use crate::Shell;
use gtk::gdk::prelude::GdkCairoContextExt;
use gtk::gdk_pixbuf::Pixbuf;
use gtk::prelude::*;
use gtk::{cairo, gdk, gio, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::Duration;

extern "C" {
    fn gdk_wayland_toplevel_set_application_id(toplevel: *mut gdk::ffi::GdkToplevel, id: *const std::ffi::c_char);
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// A test profile never makes a sound on the computer's speakers.
fn muted_by_test() -> bool {
    std::env::var_os("KALANGO_AREA_TESTE").is_some() || std::env::var("KALANGO_SONS").as_deref() == Ok("nao")
}

const MPRIS_XML: &str = r#"<node>
  <interface name="org.mpris.MediaPlayer2">
    <method name="Raise"/><method name="Quit"/>
    <property name="CanQuit" type="b" access="read"/><property name="CanRaise" type="b" access="read"/>
    <property name="HasTrackList" type="b" access="read"/><property name="Identity" type="s" access="read"/>
    <property name="DesktopEntry" type="s" access="read"/>
    <property name="SupportedUriSchemes" type="as" access="read"/><property name="SupportedMimeTypes" type="as" access="read"/>
  </interface>
  <interface name="org.mpris.MediaPlayer2.Player">
    <method name="Next"/><method name="Previous"/><method name="Pause"/><method name="PlayPause"/><method name="Stop"/><method name="Play"/>
    <method name="Seek"><arg direction="in" name="Offset" type="x"/></method>
    <method name="SetPosition"><arg direction="in" name="TrackId" type="o"/><arg direction="in" name="Position" type="x"/></method>
    <method name="OpenUri"><arg direction="in" name="Uri" type="s"/></method>
    <signal name="Seeked"><arg name="Position" type="x"/></signal>
    <property name="PlaybackStatus" type="s" access="read"/><property name="Metadata" type="a{sv}" access="read"/>
    <property name="Position" type="x" access="read"/><property name="Volume" type="d" access="read"/>
    <property name="Rate" type="d" access="read"/><property name="MinimumRate" type="d" access="read"/><property name="MaximumRate" type="d" access="read"/>
    <property name="CanGoNext" type="b" access="read"/><property name="CanGoPrevious" type="b" access="read"/>
    <property name="CanPlay" type="b" access="read"/><property name="CanPause" type="b" access="read"/>
    <property name="CanSeek" type="b" access="read"/><property name="CanControl" type="b" access="read"/>
  </interface>
</node>"#;

pub struct MusicPlayer {
    window: gtk::ApplicationWindow,
    pages: gtk::Stack,
    rail: Vec<(&'static str, gtk::Button)>,
    cover: gtk::DrawingArea,
    title: gtk::Label,
    sub: gtk::Label,
    wave: gtk::DrawingArea,
    t_now: gtk::Label,
    t_total: gtk::Label,
    fav: gtk::Button,
    play: gtk::Button,
    mix_card: gtk::Box,
    mix_title: gtk::Label,
    mix_line: gtk::Label,
    mix_play: gtk::Button,
    up_next: gtk::Box,
    songs: gtk::Box,
    lists: gtk::Box,
    favs: gtk::Box,
    search: gtk::Entry,
    empty: gtk::Label,
    library: RefCell<Vec<Track>>,
    queue: RefCell<Vec<usize>>,
    pos: Cell<usize>,
    media: RefCell<Option<gtk::MediaFile>>,
    waveform: RefCell<Vec<f32>>,
    cover_surface: RefCell<Option<cairo::ImageSurface>>,
    old_cover: RefCell<Option<cairo::ImageSurface>>,
    slide: Cell<f64>,
    mix: RefCell<Option<Mix>>,
    mpris: RefCell<Option<gio::DBusConnection>>,
    dragging: Cell<bool>,
    now_col: gtk::Box,
    measured: RefCell<Vec<(String, gtk::Widget)>>,
    right_col: gtk::Box,
    narrow: Cell<bool>,
    queue_shown: Cell<bool>,
    shell: Weak<Shell>,
}

thread_local! {
    static PLAYER: RefCell<Option<Rc<MusicPlayer>>> = const { RefCell::new(None) };
}

/// Opens the app (playing a file when one is given).
pub fn open(shell: &Rc<Shell>, file: Option<PathBuf>) {
    let p = PLAYER.with(|p| p.borrow().clone()).unwrap_or_else(|| {
        let app = gio::Application::default().and_downcast::<gtk::Application>().expect("app");
        let p = MusicPlayer::new(&app, shell);
        PLAYER.with(|x| *x.borrow_mut() = Some(p.clone()));
        p.load_library(file.clone());
        p
    });
    if !p.window.is_visible() {
        let (w, h) = crate::superficies::tamanho_de_janela(1500, 910);
        p.window.set_default_size(w, h);
    }
    p.window.present();
    if let Some(f) = file {
        p.play_file(&f);
    }
}

fn icon_button(glyph: &str, color: &str, px: i32, class: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class(class);
    b.set_child(Some(&ui_icon(glyph, color, px)));
    b.set_tooltip_text(Some(tip));
    b.set_valign(Align::Center);
    b
}

fn left(t: &str, class: &str) -> gtk::Label {
    let l = rotulo(t, class);
    l.set_xalign(0.0);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_max_width_chars(1);
    l
}

/// A cover ready to draw, from its file (cropped to a square).
fn cover_surface(p: &Path) -> Option<cairo::ImageSurface> {
    let pb = Pixbuf::from_file_at_scale(p, 840, 840, true).ok()?;
    let side = pb.width().min(pb.height());
    let pb = pb.new_subpixbuf((pb.width() - side) / 2, (pb.height() - side) / 2, side, side);
    let s = cairo::ImageSurface::create(cairo::Format::ARgb32, side, side).ok()?;
    let cr = cairo::Context::new(&s).ok()?;
    cr.set_source_pixbuf(&pb, 0.0, 0.0);
    cr.paint().ok()?;
    drop(cr);
    Some(s)
}

fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.close_path();
}

/// The cover, or KalangoOS's own when the file has none: a night sky over hills, the moon, from the track's name.
fn draw_cover(cr: &cairo::Context, s: Option<&cairo::ImageSurface>, x: f64, y: f64, side: f64, seed: &str) {
    rounded(cr, x, y, side, side, 6.0);
    cr.save().ok();
    cr.clip();
    match s {
        Some(s) => {
            let k = side / f64::from(s.width().max(1));
            cr.translate(x, y);
            cr.scale(k, k);
            let _ = cr.set_source_surface(s, 0.0, 0.0);
            cr.source().set_filter(cairo::Filter::Good);
            let _ = cr.paint();
        }
        None => {
            // (the same name, the same picture)
            let h = seed.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(u32::from(b)));
            let hue = f64::from(h % 360) / 360.0;
            let g = cairo::LinearGradient::new(x, y, x, y + side);
            let (r1, g1, b1) = hsl(hue, 0.35, 0.16);
            let (r2, g2, b2) = hsl(hue, 0.30, 0.30);
            g.add_color_stop_rgb(0.0, r1, g1, b1);
            g.add_color_stop_rgb(1.0, r2, g2, b2);
            let _ = cr.set_source(&g);
            let _ = cr.paint();
            // stars
            for i in 0..14u32 {
                let k = h.wrapping_mul(i + 3).wrapping_add(i * 977);
                cr.set_source_rgba(0.89, 0.89, 0.92, 0.55 + f64::from(k % 40) / 100.0);
                cr.arc(x + f64::from(k % 1000) / 1000.0 * side, y + f64::from((k / 1000) % 600) / 1000.0 * side, 1.2 + f64::from(k % 3), 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
            }
            // the moon and the hills
            cr.set_source_rgba(0.78, 0.80, 0.93, 0.9);
            cr.arc(x + side * 0.5, y + side * 0.62, side * 0.22, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            cr.set_source_rgb(0.08, 0.08, 0.10);
            cr.move_to(x, y + side * 0.82);
            cr.curve_to(x + side * 0.3, y + side * 0.74, x + side * 0.6, y + side * 0.88, x + side, y + side * 0.78);
            cr.line_to(x + side, y + side);
            cr.line_to(x, y + side);
            cr.close_path();
            let _ = cr.fill();
        }
    }
    cr.restore().ok();
}

fn hsl(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h * 6.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 { 0 => (c, x, 0.0), 1 => (x, c, 0.0), 2 => (0.0, c, x), 3 => (0.0, x, c), 4 => (x, 0.0, c), _ => (c, 0.0, x) };
    let m = l - c / 2.0;
    (r + m, g + m, b + m)
}

impl MusicPlayer {
    fn new(app: &gtk::Application, shell: &Rc<Shell>) -> Rc<MusicPlayer> {
        let (w, h) = crate::superficies::tamanho_de_janela(1500, 910);
        let window = gtk::ApplicationWindow::builder().application(app).title(tr("Music")).default_width(w).default_height(h).build();
        // (one window for the app, kept: closing hides it and opening shows it again — a closed GTK window shown again
        // could not be closed any more, and it kept text input from the other windows)
        window.set_hide_on_close(true);
        window.add_css_class("ed-janela");
        let sem_barra = gtk::Box::new(Orientation::Horizontal, 0);
        sem_barra.set_visible(false);
        window.set_titlebar(Some(&sem_barra));

        // the rail: the app, and its pages
        let rail_box = caixa(Orientation::Vertical, 6, "mus-trilho");
        let logo = crate::apps::imagem_kalango("musica", 28);
        logo.set_halign(Align::Center);
        rail_box.append(&logo);
        let line = gtk::Separator::new(Orientation::Horizontal);
        line.add_css_class("mus-trilho-linha");
        rail_box.append(&line);
        let mut rail = Vec::new();
        for (id, glyph, name) in [("inicio", "casa", tr("Home")), ("musicas", "musica", tr("Songs")), ("listas", "lista", tr("Lists")), ("favoritas", "estrela-n", tr("Favourites"))] {
            let b = icon_button(glyph, "#8E8F99", 20, "mus-trilho-botao", name);
            b.set_halign(Align::Center);
            crate::central::posicao_debug_sempre(&format!("musica trilho {id}"), b.upcast_ref());
            rail_box.append(&b);
            rail.push((id, b));
        }
        // the bar: the name, the search (or a request to the agent), the menu, the window's buttons
        let bar = caixa(Orientation::Horizontal, 10, "ed-barra");
        let name = rotulo(tr("Music"), "img-titulo");
        name.set_hexpand(true);
        name.set_xalign(0.0);
        bar.append(&name);
        let search = gtk::Entry::new();
        search.add_css_class("ed-pedir");
        search.set_size_request(340, 32);
        search.set_valign(Align::Center);
        search.set_placeholder_text(Some(tr("Search or ask: “something calm to code to”")));
        search.set_primary_icon_paintable(ui_icon("spark", "#A5A1E2", 13).paintable().as_ref());
        let sb = gtk::Overlay::new();
        sb.set_child(Some(&search));
        let key = rotulo("Ctrl K", "tb-atalho");
        key.set_halign(Align::End);
        key.set_valign(Align::Center);
        key.set_margin_end(8);
        key.set_can_target(false);
        sb.add_overlay(&key);
        bar.append(&sb);
        let menu = gtk::MenuButton::new();
        menu.add_css_class("calc-menu");
        menu.set_child(Some(&ui_icon("menu", "#8E8F99", 16)));
        menu.set_valign(Align::Center);
        let pop = gtk::Popover::new();
        pop.add_css_class("menu");
        let items = caixa(Orientation::Vertical, 2, "menu-lista");
        pop.set_child(Some(&items));
        menu.set_popover(Some(&pop));
        bar.append(&menu);
        bar.append(&crate::configuracoes::controles_janela());
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&bar));

        // the track playing: the cover, its name, the waveform, the controls
        let now = caixa(Orientation::Vertical, 0, "mus-agora");
        now.set_size_request(420, -1);
        now.set_valign(Align::Start);
        now.set_hexpand(false);
        let cover = gtk::DrawingArea::new();
        cover.set_content_width(420);
        cover.set_content_height(420);
        now.append(&cover);
        let title = rotulo("", "mus-titulo");
        title.set_margin_top(12);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_max_width_chars(1);
        now.append(&title);
        let sub = rotulo("", "mus-sub");
        sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
        sub.set_max_width_chars(1);
        sub.set_margin_top(4);
        now.append(&sub);
        let wave = gtk::DrawingArea::new();
        wave.set_content_height(44);
        wave.set_margin_top(12);
        wave.set_margin_start(20);
        wave.set_margin_end(20);
        wave.set_cursor_from_name(Some("pointer"));
        crate::central::posicao_debug_sempre("musica onda", wave.upcast_ref());
        now.append(&wave);
        let times = caixa(Orientation::Horizontal, 0, "");
        times.set_margin_top(6);
        let t_now = rotulo("0:00", "mus-tempo");
        t_now.set_hexpand(true);
        t_now.set_xalign(0.0);
        let t_total = rotulo("0:00", "mus-tempo");
        times.append(&t_now);
        times.append(&t_total);
        now.append(&times);
        let controls = caixa(Orientation::Horizontal, 26, "");
        controls.set_halign(Align::Center);
        controls.set_margin_top(18);
        let fav = icon_button("estrela-n", "#8E8F99", 18, "mus-controle", tr("Favourite"));
        let prev = icon_button("anterior", "#E2E2E6", 18, "mus-controle", tr("Previous"));
        let play = icon_button("tocar", "#1C1D24", 20, "mus-tocar", tr("Play"));
        let next = icon_button("proxima", "#E2E2E6", 18, "mus-controle", tr("Next"));
        let qb = icon_button("lista", "#E2E2E6", 18, "mus-controle", tr("Up next"));
        for (b, n) in [(&fav, "favorita"), (&prev, "anterior"), (&play, "tocar"), (&next, "proxima"), (&qb, "fila")] {
            crate::central::posicao_debug_sempre(&format!("musica {n}"), b.upcast_ref());
            controls.append(b);
        }
        now.append(&controls);

        // on the right: the agent's mix and what comes next
        let right = caixa(Orientation::Vertical, 0, "");
        right.set_hexpand(true);
        let mix_card = caixa(Orientation::Horizontal, 14, "mus-mix");
        let spark = caixa(Orientation::Horizontal, 0, "mus-mix-icone");
        let si = ui_icon("spark", "#A5A1E2", 18);
        si.set_hexpand(true);
        spark.append(&si);
        spark.set_hexpand(false);
        spark.set_valign(Align::Center);
        mix_card.append(&spark);
        let mt = caixa(Orientation::Vertical, 2, "");
        mt.set_hexpand(true);
        mt.set_valign(Align::Center);
        let mix_title = left(tr("Making a mix for now…"), "mus-mix-titulo");
        let mix_line = left(tr("the local model picks from your music"), "mus-mix-linha");
        mt.append(&mix_title);
        mt.append(&mix_line);
        mix_card.append(&mt);
        let mix_play = gtk::Button::new();
        mix_play.add_css_class("mus-mix-tocar");
        let mpc = caixa(Orientation::Horizontal, 6, "");
        mpc.append(&ui_icon("tocar", "#1C1D24", 12));
        mpc.append(&rotulo(tr("Play"), ""));
        mix_play.set_child(Some(&mpc));
        mix_play.set_valign(Align::Center);
        mix_play.set_sensitive(false);
        crate::central::posicao_debug_sempre("musica mix tocar", mix_play.upcast_ref());
        mix_card.append(&mix_play);
        right.append(&mix_card);
        let head = left(tr("UP NEXT"), "mus-secao");
        head.set_margin_top(22);
        right.append(&head);
        let up_next = caixa(Orientation::Vertical, 2, "");
        up_next.set_margin_top(8);
        let s1 = gtk::ScrolledWindow::new();
        s1.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        s1.set_vexpand(true);
        s1.set_child(Some(&up_next));
        right.append(&s1);
        let home = caixa(Orientation::Horizontal, 24, "mus-pagina");
        home.append(&now);
        home.append(&right);

        // the other pages: all the songs, the lists, the favourites
        let page = |title: &str| -> (gtk::Box, gtk::Box) {
            let col = caixa(Orientation::Vertical, 0, "mus-pagina");
            col.append(&left(title, "mus-pagina-titulo"));
            let list = caixa(Orientation::Vertical, 2, "");
            list.set_margin_top(14);
            let s = gtk::ScrolledWindow::new();
            s.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
            s.set_vexpand(true);
            s.set_child(Some(&list));
            col.append(&s);
            (col, list)
        };
        let (songs_page, songs) = page(tr("Songs"));
        let (lists_page, lists) = page(tr("Lists"));
        let (favs_page, favs) = page(tr("Favourites"));
        let pages = gtk::Stack::new();
        pages.set_transition_type(gtk::StackTransitionType::Crossfade);
        pages.set_transition_duration(140);
        pages.set_hhomogeneous(false);
        // (the page asks for no size of its own: the meter below fits the cover to the room there is)
        let home_room = gtk::ScrolledWindow::new();
        home_room.set_policy(gtk::PolicyType::External, gtk::PolicyType::External);
        home_room.set_child(Some(&home));
        pages.add_named(&home_room, Some("inicio"));
        pages.add_named(&songs_page, Some("musicas"));
        pages.add_named(&lists_page, Some("listas"));
        pages.add_named(&favs_page, Some("favoritas"));
        let empty = rotulo("", "img-vazio");
        empty.set_wrap(true);
        empty.set_max_width_chars(1);
        empty.set_visible(false);
        let body = gtk::Overlay::new();
        body.set_child(Some(&pages));
        body.add_overlay(&empty);
        // (the window measured: the cover as big as there is room for, up to the design's 420)
        let meter = gtk::DrawingArea::new();
        meter.set_can_target(false);
        body.add_overlay(&meter);
        body.set_measure_overlay(&meter, false);
        let main = caixa(Orientation::Vertical, 0, "");
        main.set_hexpand(true);
        main.append(&handle);
        main.append(&body);
        let root = caixa(Orientation::Horizontal, 0, "");
        root.append(&rail_box);
        root.append(&main);
        window.set_child(Some(&root));

        let p = Rc::new(MusicPlayer {
            window, pages, rail, cover, title, sub, wave, t_now, t_total, fav, play, mix_card, mix_title, mix_line, mix_play, up_next, songs, lists, favs,
            search, empty, library: RefCell::default(), queue: RefCell::default(), pos: Cell::new(0), media: RefCell::default(), waveform: RefCell::default(),
            cover_surface: RefCell::default(), old_cover: RefCell::default(), slide: Cell::new(0.0), mix: RefCell::default(), mpris: RefCell::default(),
            dragging: Cell::new(false), now_col: now, measured: RefCell::default(), right_col: right, narrow: Cell::new(false), queue_shown: Cell::new(false), shell: Rc::downgrade(shell),
        });
        p.wire(&prev, &next, &qb, &items, &pop);
        {
            let mut m = p.measured.borrow_mut();
            m.push(("musica onda".into(), p.wave.clone().upcast()));
            for (b, n) in [(&p.fav, "favorita"), (&prev, "anterior"), (&p.play, "tocar"), (&next, "proxima"), (&qb, "fila")] {
                m.push((format!("musica {n}"), b.clone().upcast()));
            }
        }
        let w = Rc::downgrade(&p);
        meter.connect_resize(move |_, width, height| {
            if let Some(p) = w.upgrade() {
                p.fit(width, height);
            }
        });
        p
    }

    fn wire(self: &Rc<Self>, prev: &gtk::Button, next: &gtk::Button, qb: &gtk::Button, items: &gtk::Box, pop: &gtk::Popover) {
        for (id, b) in &self.rail {
            let (id, w) = (*id, Rc::downgrade(self));
            b.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    p.page(id);
                }
            });
        }
        self.page("inicio");
        let w = Rc::downgrade(self);
        self.cover.set_draw_func(move |_, cr, width, height| {
            let Some(p) = w.upgrade() else { return };
            let side = f64::from(width.min(height));
            let seed = p.current().map(|t| t.label()).unwrap_or_default();
            let s = p.slide.get();
            if s.abs() > 0.001 {
                // the old cover leaves, the new one comes in (the same side)
                draw_cover(cr, p.old_cover.borrow().as_ref(), -s.signum() * (1.0 - s.abs()) * side, 0.0, side, "");
                draw_cover(cr, p.cover_surface.borrow().as_ref(), s * side, 0.0, side, &seed);
            } else {
                draw_cover(cr, p.cover_surface.borrow().as_ref(), 0.0, 0.0, side, &seed);
            }
        });
        let w = Rc::downgrade(self);
        self.wave.set_draw_func(move |_, cr, width, height| {
            if let Some(p) = w.upgrade() {
                p.draw_wave(cr, f64::from(width), f64::from(height));
            }
        });
        // the waveform: a click or a drag goes there
        let drag = gtk::GestureDrag::new();
        let w = Rc::downgrade(self);
        drag.connect_drag_begin(move |_, x, _| {
            if let Some(p) = w.upgrade() {
                p.dragging.set(true);
                p.seek_to_x(x);
            }
        });
        let w = Rc::downgrade(self);
        drag.connect_drag_update(move |g, dx, _| {
            if let (Some(p), Some((x, _))) = (w.upgrade(), g.start_point()) {
                p.seek_to_x(x + dx);
            }
        });
        let w = Rc::downgrade(self);
        drag.connect_drag_end(move |_, _, _| {
            if let Some(p) = w.upgrade() {
                p.dragging.set(false);
            }
        });
        self.wave.add_controller(drag);
        let w = Rc::downgrade(self);
        self.play.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.toggle();
            }
        });
        let w = Rc::downgrade(self);
        prev.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.previous();
            }
        });
        let w = Rc::downgrade(self);
        next.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.next();
            }
        });
        let w = Rc::downgrade(self);
        qb.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                // (a narrow window shows the track or what comes next: this button goes between them)
                if p.narrow.get() && p.pages.visible_child_name().as_deref() == Some("inicio") {
                    p.queue_shown.set(!p.queue_shown.get());
                    p.fit_columns();
                }
                p.page("inicio");
            }
        });
        let w = Rc::downgrade(self);
        self.fav.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.toggle_favourite();
            }
        });
        let w = Rc::downgrade(self);
        self.mix_play.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.play_mix();
            }
        });
        // the search: typing filters the songs; Enter with a few words asks the agent for a mix
        let w = Rc::downgrade(self);
        self.search.connect_changed(move |e| {
            if let Some(p) = w.upgrade() {
                if !e.text().is_empty() {
                    p.page("musicas");
                }
                p.fill_songs();
            }
        });
        let w = Rc::downgrade(self);
        self.search.connect_activate(move |e| {
            let t = e.text().trim().to_string();
            let Some(p) = w.upgrade() else { return };
            if t.split_whitespace().count() >= 3 {
                e.set_text("");
                p.page("inicio");
                p.make_mix(Some(t));
            } else if let Some(i) = p.filtered().first().copied() {
                p.play_from(p.filtered(), i);
            }
        });
        // keys: space plays or pauses, ← → 5 s, Ctrl+← → the previous and the next, Ctrl+K the search
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, k, _, m| {
            let Some(p) = w.upgrade() else { return glib::Propagation::Proceed };
            let ctrl = m.contains(gdk::ModifierType::CONTROL_MASK);
            // (typing in the search is typing: its focus is the text inside it)
            let typing = gtk::prelude::GtkWindowExt::focus(&p.window).is_some_and(|f| f.is_ancestor(&p.search) || f == *p.search.upcast_ref::<gtk::Widget>());
            if typing && !(ctrl && k == gdk::Key::k) {
                return glib::Propagation::Proceed;
            }
            match k {
                gdk::Key::k if ctrl => {
                    p.search.grab_focus();
                }
                gdk::Key::space => p.toggle(),
                gdk::Key::Right if ctrl => p.next(),
                gdk::Key::Left if ctrl => p.previous(),
                gdk::Key::Right => p.seek_by(5.0),
                gdk::Key::Left => p.seek_by(-5.0),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.window.add_controller(keys);
        // the menu
        let add = |label: &str, f: Box<dyn Fn(&Rc<MusicPlayer>)>| {
            let b = gtk::Button::with_label(label);
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            b.add_css_class("menu-item");
            let (w, pop) = (Rc::downgrade(self), pop.clone());
            b.connect_clicked(move |_| {
                pop.popdown();
                if let Some(p) = w.upgrade() {
                    f(&p);
                }
            });
            items.append(&b);
        };
        add(tr("Read the Music folder again"), Box::new(|p| p.load_library(None)));
        add(tr("Show the Music folder"), Box::new(|p| {
            if let Some(s) = p.shell.upgrade() {
                s.abrir_arquivos(Some(&lib::music_dir()));
            }
        }));
        // closing the window stops the music (the app has no life of its own behind it)
        let w = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(p) = w.upgrade() {
                if let Some(m) = p.media.borrow().as_ref() {
                    m.pause();
                }
                p.mpris_changed();
            }
            glib::Propagation::Proceed
        });
        let app_id = |j: &gtk::ApplicationWindow| {
            if let Some(s) = j.surface().filter(|s| s.type_().name().contains("Wayland")) {
                let id = std::ffi::CString::new("os.kalango.Musica").unwrap();
                unsafe { gdk_wayland_toplevel_set_application_id(s.as_ptr() as *mut gdk::ffi::GdkToplevel, id.as_ptr()) };
            }
        };
        self.window.connect_realize(move |j| app_id(j));
        self.window.connect_map(move |j| {
            app_id(j);
            // (the keys are the player's: the search is focused only when asked, Ctrl+K)
            let j = j.clone();
            glib::idle_add_local_once(move || gtk::prelude::GtkWindowExt::set_focus(&j, None::<&gtk::Widget>));
        });
        // the progress, four times a second
        let w = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(250), move || {
            let Some(p) = w.upgrade() else { return glib::ControlFlow::Break };
            p.tick();
            glib::ControlFlow::Continue
        });
        self.serve_mpris();
    }

    /// Fits the home page to the body: the cover below the design's 420 when the window is short or narrow (the
    /// controls always in view), and on a narrow window one column at a time.
    fn fit(&self, width: i32, height: i32) {
        // (below the cover: the name, the artist, the waveform, the times and the controls — about 260 px with the page's padding)
        let narrow = width < 820;
        let room_w = if narrow { width - 48 } else { (f64::from(width - 72) * 0.4) as i32 };
        let side = (height - 260).min(room_w).clamp(160, 420);
        if self.cover.content_width() != side {
            self.cover.set_content_width(side);
            self.cover.set_content_height(side);
            self.now_col.set_size_request(side, -1);
            // (for the session test: the controls moved with the cover, measured again)
            for (name, w) in self.measured.borrow().iter() {
                crate::central::posicao_debug(name, w);
            }
        }
        if narrow != self.narrow.get() {
            self.narrow.set(narrow);
            self.queue_shown.set(false);
            self.fit_columns();
        }
        if debug() && self.cover.content_width() == side {
            eprintln!("debug musica: medida {width}×{height}, capa {side}{}", if narrow { ", estreita" } else { "" });
        }
    }

    fn fit_columns(&self) {
        let (narrow, queue) = (self.narrow.get(), self.queue_shown.get());
        self.now_col.set_visible(!narrow || !queue);
        self.right_col.set_visible(!narrow || queue);
        self.now_col.set_hexpand(narrow);
        self.now_col.set_halign(if narrow { Align::Center } else { Align::Fill });
    }

    fn page(&self, id: &str) {
        self.pages.set_visible_child_name(id);
        for (x, b) in &self.rail {
            if *x == id {
                b.add_css_class("ativo");
            } else {
                b.remove_css_class("ativo");
            }
        }
        if debug() {
            eprintln!("debug musica: página {id}");
        }
    }

    fn current(&self) -> Option<Track> {
        let q = self.queue.borrow();
        let i = *q.get(self.pos.get())?;
        self.library.borrow().get(i).cloned()
    }

    // ------------------------------------------------------------------------------------------- the library

    fn load_library(self: &Rc<Self>, then_play: Option<PathBuf>) {
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(lib::scan, move |tracks: Vec<Track>| {
            let Some(p) = w.upgrade() else { return };
            if debug() {
                eprintln!("debug musica: biblioteca ({} faixas em {})", tracks.len(), lib::music_dir().display());
            }
            let n = tracks.len();
            *p.library.borrow_mut() = tracks;
            p.empty.set_visible(n == 0);
            p.mix_card.set_visible(n > 0);
            p.empty.set_text(&trf("No music in {path} yet: songs put there show up here.", &[("path", lib::music_dir().display().to_string())]));
            if p.queue.borrow().is_empty() && n > 0 {
                *p.queue.borrow_mut() = (0..n).collect();
                p.show_current(0.0);
            }
            p.fill_songs();
            p.fill_lists();
            p.fill_favs();
            p.fill_up_next();
            if let Some(f) = then_play {
                p.play_file(&f);
            }
            if n > 0 && p.mix.borrow().is_none() {
                p.make_mix(None);
            }
        });
    }

    fn filtered(&self) -> Vec<usize> {
        let q = self.search.text().to_lowercase();
        self.library.borrow().iter().enumerate().filter(|(_, t)| q.is_empty() || [&t.title, &t.artist, &t.album, &t.genre].iter().any(|s| s.to_lowercase().contains(&q))
            || t.label().to_lowercase().contains(&q)).map(|(i, _)| i).collect()
    }

    /// A song's row: a click plays it (`on_click`); its ⋮ menu plays it now or next, takes it out of what comes next
    /// (`queued`: its place there) or shows it in its folder.
    fn row(self: &Rc<Self>, i: usize, on_click: Rc<dyn Fn(&Rc<MusicPlayer>)>, current: bool, queued: Option<usize>) -> gtk::Box {
        let t = self.library.borrow()[i].clone();
        let outer = caixa(Orientation::Horizontal, 0, "mus-linha-caixa");
        if current {
            outer.add_css_class("atual");
        }
        let b = gtk::Button::new();
        b.add_css_class("mus-linha");
        b.set_hexpand(true);
        let r = caixa(Orientation::Horizontal, 12, "");
        let thumb = gtk::DrawingArea::new();
        thumb.set_content_width(36);
        thumb.set_content_height(36);
        let surf = t.cover.as_ref().and_then(|c| {
            let pb = Pixbuf::from_file_at_scale(c, 72, 72, false).ok()?;
            let s = cairo::ImageSurface::create(cairo::Format::ARgb32, pb.width(), pb.height()).ok()?;
            let cr = cairo::Context::new(&s).ok()?;
            cr.set_source_pixbuf(&pb, 0.0, 0.0);
            cr.paint().ok()?;
            drop(cr);
            Some(s)
        });
        let seed = t.label();
        thumb.set_draw_func(move |_, cr, w, h| draw_cover(cr, surf.as_ref(), 0.0, 0.0, f64::from(w.min(h)), &seed));
        r.append(&thumb);
        let txt = caixa(Orientation::Vertical, 1, "");
        txt.set_hexpand(true);
        txt.set_valign(Align::Center);
        txt.append(&left(&t.label(), "mus-linha-titulo"));
        txt.append(&left(if t.artist.is_empty() { tr("unknown artist") } else { &t.artist }, "mus-linha-sub"));
        r.append(&txt);
        let d = rotulo(&lib::clock(t.seconds), "mus-duracao");
        d.set_valign(Align::Center);
        r.append(&d);
        b.set_child(Some(&r));
        let w = Rc::downgrade(self);
        let play = on_click.clone();
        b.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                play(&p);
            }
        });
        outer.append(&b);
        let more = gtk::MenuButton::new();
        more.add_css_class("mus-mais");
        more.set_child(Some(&ui_icon("pontos-v", "#8E8F99", 14)));
        more.set_valign(Align::Center);
        more.set_tooltip_text(Some(tr("More")));
        let pop = gtk::Popover::new();
        pop.add_css_class("menu");
        let items = caixa(Orientation::Vertical, 2, "menu-lista");
        let mut acts: Vec<(&str, &str, Rc<dyn Fn(&Rc<MusicPlayer>)>)> = vec![("agora", tr("Play now"), on_click)];
        acts.push(("depois", tr("Play next"), Rc::new(move |p: &Rc<MusicPlayer>| p.queue_next(i, queued))));
        if let Some(k) = queued {
            acts.push(("tirar", tr("Take out of up next"), Rc::new(move |p: &Rc<MusicPlayer>| p.unqueue(k))));
        }
        let path = t.path.clone();
        acts.push(("pasta", tr("Show in the folder"), Rc::new(move |p: &Rc<MusicPlayer>| {
            if let Some(s) = p.shell.upgrade() {
                s.abrir_arquivos(Some(&path));
            }
        })));
        for (id, label, f) in acts {
            let it = gtk::Button::with_label(label);
            if let Some(l) = it.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            it.add_css_class("menu-item");
            if queued.is_some() {
                crate::central::posicao_debug_sempre(&format!("musica menu {id}"), it.upcast_ref());
            }
            let (w, pop) = (Rc::downgrade(self), pop.clone());
            it.connect_clicked(move |_| {
                pop.popdown();
                if let Some(p) = w.upgrade() {
                    f(&p);
                }
            });
            items.append(&it);
        }
        pop.set_child(Some(&items));
        more.set_popover(Some(&pop));
        outer.append(&more);
        outer
    }

    /// Puts song `i` right after the one playing (moved there when it was already in what comes next, at `from`).
    fn queue_next(self: &Rc<Self>, i: usize, from: Option<usize>) {
        {
            let mut q = self.queue.borrow_mut();
            let pos = self.pos.get();
            if let Some(k) = from.filter(|k| *k < q.len() && *k != pos) {
                q.remove(k);
                if k < pos {
                    self.pos.set(pos - 1);
                }
            }
            let at = (self.pos.get() + 1).min(q.len());
            q.insert(at, i);
        }
        if debug() {
            eprintln!("debug musica: a seguir «{}»", self.library.borrow()[i].label());
        }
        self.fill_up_next();
        self.mpris_changed();
    }

    /// Takes place `k` out of what comes next.
    fn unqueue(self: &Rc<Self>, k: usize) {
        let label = {
            let mut q = self.queue.borrow_mut();
            if k >= q.len() || k == self.pos.get() {
                return;
            }
            let i = q.remove(k);
            if k < self.pos.get() {
                self.pos.set(self.pos.get() - 1);
            }
            self.library.borrow()[i].label()
        };
        if debug() {
            eprintln!("debug musica: fora da fila «{label}» ({} na fila)", self.queue.borrow().len());
        }
        self.fill_up_next();
        self.mpris_changed();
    }

    fn clear(b: &gtk::Box) {
        while let Some(c) = b.first_child() {
            b.remove(&c);
        }
    }

    fn fill_songs(self: &Rc<Self>) {
        Self::clear(&self.songs);
        let list = self.filtered();
        let cur = self.current().map(|t| t.path);
        for (k, &i) in list.iter().enumerate().take(500) {
            let list2 = list.clone();
            let is_cur = cur.as_ref() == Some(&self.library.borrow()[i].path);
            let b = self.row(i, Rc::new(move |p: &Rc<MusicPlayer>| p.play_from(list2.clone(), i)), is_cur, None);
            crate::central::posicao_debug_sempre(&format!("musica faixa {}", k + 1), b.upcast_ref());
            self.songs.append(&b);
        }
    }

    fn fill_favs(self: &Rc<Self>) {
        Self::clear(&self.favs);
        let favs = lib::favourites();
        let idx: Vec<usize> = favs.iter().filter_map(|f| self.library.borrow().iter().position(|t| &t.path == f)).collect();
        if idx.is_empty() {
            self.favs.append(&left(tr("The songs you star show up here."), "mus-linha-sub"));
        }
        for &i in &idx {
            let list = idx.clone();
            self.favs.append(&self.row(i, Rc::new(move |p: &Rc<MusicPlayer>| p.play_from(list.clone(), i)), false, None));
        }
    }

    fn fill_lists(self: &Rc<Self>) {
        Self::clear(&self.lists);
        let mixes = lib::mixes();
        if mixes.is_empty() {
            self.lists.append(&left(tr("The agent's mixes you play are kept here."), "mus-linha-sub"));
        }
        for m in mixes {
            let b = gtk::Button::new();
            b.add_css_class("mus-linha");
            let r = caixa(Orientation::Horizontal, 12, "");
            let ic = caixa(Orientation::Horizontal, 0, "mus-mix-icone");
            let si = ui_icon("spark", "#A5A1E2", 16);
            si.set_hexpand(true);
            ic.append(&si);
            ic.set_hexpand(false);
            r.append(&ic);
            let txt = caixa(Orientation::Vertical, 1, "");
            txt.set_hexpand(true);
            txt.append(&left(&m.title, "mus-linha-titulo"));
            txt.append(&left(&trf("{n} songs · {line}", &[("n", m.tracks.len().to_string()), ("line", m.line.clone())]), "mus-linha-sub"));
            r.append(&txt);
            b.set_child(Some(&r));
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    let idx: Vec<usize> = m.tracks.iter().filter_map(|f| p.library.borrow().iter().position(|t| &t.path == f)).collect();
                    if let Some(&first) = idx.first() {
                        p.play_from(idx, first);
                    }
                }
            });
            self.lists.append(&b);
        }
    }

    fn fill_up_next(self: &Rc<Self>) {
        Self::clear(&self.up_next);
        let q = self.queue.borrow().clone();
        let pos = self.pos.get();
        for (k, &i) in q.iter().enumerate().skip(pos + 1).take(40) {
            // (the next one stands out, as in the design)
            let b = self.row(i, Rc::new(move |p: &Rc<MusicPlayer>| p.play_at(k)), k == pos + 1, Some(k));
            crate::central::posicao_debug_sempre(&format!("musica seguir {}", k - pos), b.upcast_ref());
            self.up_next.append(&b);
        }
        if q.len() <= pos + 1 {
            self.up_next.append(&left(tr("Nothing after this one."), "mus-linha-sub"));
        }
    }

    // ------------------------------------------------------------------------------------------- playing

    fn play_file(self: &Rc<Self>, f: &Path) {
        let i = self.library.borrow().iter().position(|t| t.path == f);
        let i = match i {
            Some(i) => i,
            None => {
                // a file from elsewhere: read and played
                let Some(t) = lib::read_track(f) else { return };
                self.library.borrow_mut().push(t);
                self.library.borrow().len() - 1
            }
        };
        let all: Vec<usize> = (0..self.library.borrow().len()).collect();
        self.play_from(all, i);
    }

    /// Plays `i` with `list` as the queue (from it on).
    fn play_from(self: &Rc<Self>, list: Vec<usize>, i: usize) {
        let k = list.iter().position(|x| *x == i).unwrap_or(0);
        *self.queue.borrow_mut() = list;
        self.play_at(k);
    }

    fn play_at(self: &Rc<Self>, k: usize) {
        let from = if k >= self.pos.get() { 1.0 } else { -1.0 };
        self.pos.set(k);
        let Some(t) = self.current() else { return };
        if let Some(old) = self.media.borrow_mut().take() {
            old.pause();
        }
        let m = gtk::MediaFile::for_filename(&t.path);
        if muted_by_test() {
            m.set_muted(true);
        }
        let w = Rc::downgrade(self);
        m.connect_ended_notify(move |m| {
            if m.is_ended() {
                if let Some(p) = w.upgrade() {
                    p.next();
                }
            }
        });
        let w = Rc::downgrade(self);
        m.connect_error_notify(move |m| {
            if let (Some(p), Some(e)) = (w.upgrade(), m.error()) {
                eprintln!("kalango-shell: música: {e}");
                p.sub.set_text(&trf("This song could not be played: {error}", &[("error", e.to_string())]));
            }
        });
        m.play();
        *self.media.borrow_mut() = Some(m);
        if debug() {
            eprintln!("debug musica: tocando «{}» — {} ({} de {}){}", t.label(), t.artist, k + 1, self.queue.borrow().len(), if muted_by_test() { " · mudo (teste)" } else { "" });
        }
        self.show_current(from);
        self.fill_up_next();
        self.fill_songs();
        self.mpris_changed();
    }

    /// Shows the track in view: its cover slides in, its name, its waveform (read in the background).
    fn show_current(self: &Rc<Self>, from: f64) {
        let Some(t) = self.current() else { return };
        self.title.set_text(&t.label());
        let parts: Vec<&str> = [t.artist.as_str(), t.album.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
        self.sub.set_text(&if parts.is_empty() { tr("unknown artist").to_string() } else { parts.join(" · ") });
        self.t_total.set_text(&lib::clock(t.seconds));
        let fav = lib::favourites().contains(&t.path);
        set_icon(self.fav.child().and_downcast_ref::<gtk::Image>().unwrap_or(&gtk::Image::new()), "estrela-n", if fav { "#E5C98C" } else { "#8E8F99" });
        // the cover slides (220 ms, easing out)
        *self.old_cover.borrow_mut() = self.cover_surface.borrow().clone();
        *self.cover_surface.borrow_mut() = t.cover.as_deref().and_then(cover_surface);
        if from != 0.0 {
            self.slide.set(from);
            let start = glib::monotonic_time();
            let w = Rc::downgrade(self);
            self.cover.add_tick_callback(move |_, _| {
                let Some(p) = w.upgrade() else { return glib::ControlFlow::Break };
                let x = ((glib::monotonic_time() - start) as f64 / 260_000.0).min(1.0);
                p.slide.set(from * (1.0 - (1.0 - (1.0 - x).powi(3))));
                p.cover.queue_draw();
                if x >= 1.0 { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
            });
        }
        self.cover.queue_draw();
        *self.waveform.borrow_mut() = vec![0.25; lib::BARS];
        self.wave.queue_draw();
        let w = Rc::downgrade(self);
        let path = t.path.clone();
        crate::lateral::em_segundo_plano(move || lib::waveform(&t), move |v: Vec<f32>| {
            let Some(p) = w.upgrade() else { return };
            if p.current().map(|t| t.path) == Some(path) {
                if debug() {
                    eprintln!("debug musica: onda ({} barras)", v.len());
                }
                *p.waveform.borrow_mut() = v;
                p.wave.queue_draw();
            }
        });
    }

    fn playing(&self) -> bool {
        self.media.borrow().as_ref().is_some_and(|m| m.is_playing())
    }

    fn toggle(self: &Rc<Self>) {
        let m = self.media.borrow().clone();
        match m {
            Some(m) if m.is_playing() => m.pause(),
            Some(m) => m.play(),
            None => {
                if !self.queue.borrow().is_empty() {
                    self.play_at(self.pos.get());
                }
                return;
            }
        }
        if debug() {
            eprintln!("debug musica: {}", if self.playing() { "tocando" } else { "pausado" });
        }
        self.mpris_changed();
    }

    fn next(self: &Rc<Self>) {
        let n = self.queue.borrow().len();
        if self.pos.get() + 1 < n {
            self.play_at(self.pos.get() + 1);
        } else if let Some(m) = self.media.borrow().as_ref() {
            m.pause();
        }
    }

    fn previous(self: &Rc<Self>) {
        // (a few seconds in: back to its start)
        let ts = self.media.borrow().as_ref().map(|m| m.timestamp()).unwrap_or(0);
        if ts > 3_000_000 || self.pos.get() == 0 {
            if let Some(m) = self.media.borrow().as_ref() {
                m.seek(0);
            }
            return;
        }
        self.play_at(self.pos.get() - 1);
    }

    fn duration_us(&self) -> i64 {
        let d = self.media.borrow().as_ref().map(|m| m.duration()).unwrap_or(0);
        if d > 0 { d } else { self.current().map(|t| t.seconds as i64 * 1_000_000).unwrap_or(0) }
    }

    fn seek_to_x(self: &Rc<Self>, x: f64) {
        let w = f64::from(self.wave.width()).max(1.0);
        let d = self.duration_us();
        if d <= 0 {
            return;
        }
        let to = ((x / w).clamp(0.0, 1.0) * d as f64) as i64;
        if let Some(m) = self.media.borrow().as_ref() {
            m.seek(to);
        }
        if debug() {
            eprintln!("debug musica: posição {}s", to / 1_000_000);
        }
        self.tick();
    }

    fn seek_by(self: &Rc<Self>, s: f64) {
        let m = self.media.borrow().clone();
        if let Some(m) = m {
            let to = (m.timestamp() + (s * 1_000_000.0) as i64).clamp(0, self.duration_us().max(0));
            m.seek(to);
            self.tick();
        }
    }

    fn tick(&self) {
        let (ts, d) = (self.media.borrow().as_ref().map(|m| m.timestamp()).unwrap_or(0), self.duration_us());
        self.t_now.set_text(&lib::clock((ts / 1_000_000) as u64));
        if d > 0 {
            self.t_total.set_text(&lib::clock((d / 1_000_000) as u64));
        }
        set_icon(self.play.child().and_downcast_ref::<gtk::Image>().unwrap_or(&gtk::Image::new()), if self.playing() { "pausa" } else { "tocar" }, "#1C1D24");
        self.wave.queue_draw();
    }

    /// The design's waveform: thin rounded bars, the part played in lavender.
    fn draw_wave(&self, cr: &cairo::Context, w: f64, h: f64) {
        let v = self.waveform.borrow();
        if v.is_empty() {
            return;
        }
        let d = self.duration_us();
        let ts = self.media.borrow().as_ref().map(|m| m.timestamp()).unwrap_or(0);
        let played = if d > 0 { ts as f64 / d as f64 } else { 0.0 };
        // (one bar each 6 px, as in the design: the peaks resampled to fit)
        let count = ((w / 6.0).floor() as usize).clamp(8, v.len());
        let bars: Vec<f32> = (0..count).map(|k| {
            let (a, b) = (k * v.len() / count, ((k + 1) * v.len() / count).max(k * v.len() / count + 1));
            v[a..b.min(v.len())].iter().copied().fold(0.0, f32::max)
        }).collect();
        let n = count as f64;
        let step = w / n;
        let bw = (step * 0.42).max(1.5);
        for (i, a) in bars.iter().enumerate() {
            let x = i as f64 * step + (step - bw) / 2.0;
            let bh = (f64::from(*a) * h).max(3.0);
            let on = (i as f64 + 0.5) / n <= played;
            let (r, g, b) = crate::tema::rgb(if on { "#A5A1E2" } else { "#4A4B57" });
            cr.set_source_rgb(r, g, b);
            rounded(cr, x, (h - bh) / 2.0, bw, bh, bw / 2.0);
            let _ = cr.fill();
        }
    }

    fn toggle_favourite(self: &Rc<Self>) {
        let Some(t) = self.current() else { return };
        let on = !lib::favourites().contains(&t.path);
        lib::set_favourite(&t.path, on);
        set_icon(self.fav.child().and_downcast_ref::<gtk::Image>().unwrap_or(&gtk::Image::new()), "estrela-n", if on { "#E5C98C" } else { "#8E8F99" });
        self.fill_favs();
        if debug() {
            eprintln!("debug musica: favorita «{}» {on}", t.label());
        }
    }

    // ------------------------------------------------------------------------------------------- the agent's mix

    /// Asks the local model for a mix: for a request, or for this moment of the day.
    fn make_mix(self: &Rc<Self>, request: Option<String>) {
        let lib_tracks = self.library.borrow().clone();
        if lib_tracks.is_empty() {
            return;
        }
        let hour = glib::DateTime::now_local().map(|d| d.hour()).unwrap_or(12);
        let moment = match hour { 0..=5 => "madrugada", 6..=11 => "manhã", 12..=17 => "tarde", _ => "noite" };
        self.mix_title.set_text(if request.is_some() { tr("Making your mix…") } else { tr("Making a mix for now…") });
        self.mix_line.set_text(tr("the local model picks from your music"));
        self.mix_play.set_sensitive(false);
        let input = serde_json::json!({
            "pedido": request.clone().unwrap_or_default(), "hora": moment,
            "faixas": lib_tracks.iter().enumerate().map(|(i, t)| serde_json::json!({"id": i, "titulo": t.label(), "artista": t.artist, "album": t.album, "genero": t.genre, "duracao": t.seconds})).collect::<Vec<_>>(),
        }).to_string();
        let bin = crate::voz::agent_binary();
        let w = Rc::downgrade(self);
        let asked = request.is_some();
        crate::lateral::em_segundo_plano(move || {
            use std::io::Write;
            let mut c = std::process::Command::new(&bin).arg("musica-mix").stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().map_err(|e| e.to_string())?;
            c.stdin.take().ok_or("stdin")?.write_all(input.as_bytes()).map_err(|e| e.to_string())?;
            let o = c.wait_with_output().map_err(|e| e.to_string())?;
            serde_json::from_slice::<serde_json::Value>(&o.stdout).map_err(|_| String::from_utf8_lossy(&o.stderr).trim().lines().last().unwrap_or("").to_string())
        }, move |r: Result<serde_json::Value, String>| {
            let Some(p) = w.upgrade() else { return };
            match r {
                Ok(v) => {
                    let lib_now = p.library.borrow().clone();
                    let tracks: Vec<PathBuf> = v["faixas"].as_array().into_iter().flatten().filter_map(|x| x.as_u64()).filter_map(|i| lib_now.get(i as usize).map(|t| t.path.clone())).collect();
                    if tracks.is_empty() {
                        p.mix_title.set_text(tr("No mix this time"));
                        p.mix_line.set_text(tr("the model chose nothing from the library"));
                        return;
                    }
                    let m = Mix { title: v["titulo"].as_str().unwrap_or("Mix").to_string(), line: v["descricao"].as_str().unwrap_or("").to_string(), tracks };
                    p.mix_title.set_text(&m.title);
                    p.mix_line.set_text(&trf("made by the agent: {n} songs · {line}", &[("n", m.tracks.len().to_string()), ("line", m.line.clone())]));
                    p.mix_play.set_sensitive(true);
                    if debug() {
                        eprintln!("debug musica: mix «{}» ({} faixas)", m.title, m.tracks.len());
                    }
                    *p.mix.borrow_mut() = Some(m);
                    // a mix asked for plays at once
                    if asked {
                        p.play_mix();
                    }
                }
                Err(e) => {
                    p.mix_title.set_text(tr("The mix could not be made"));
                    p.mix_line.set_text(&e);
                }
            }
        });
    }

    fn play_mix(self: &Rc<Self>) {
        let Some(m) = self.mix.borrow().clone() else { return };
        let idx: Vec<usize> = m.tracks.iter().filter_map(|f| self.library.borrow().iter().position(|t| &t.path == f)).collect();
        if let Some(&first) = idx.first() {
            lib::keep_mix(&m);
            self.fill_lists();
            self.play_from(idx, first);
        }
    }

    // ------------------------------------------------------------------------------------------- MPRIS

    fn metadata(&self) -> glib::Variant {
        let d = glib::VariantDict::new(None);
        if let Some(t) = self.current() {
            let id = format!("/os/kalango/Musica/faixa/{}", self.pos.get());
            d.insert_value("mpris:trackid", &glib::variant::ObjectPath::try_from(id).map(|o| o.to_variant()).unwrap_or_else(|_| "/".to_variant()));
            d.insert_value("xesam:title", &t.label().to_variant());
            d.insert_value("xesam:artist", &vec![t.artist.clone()].to_variant());
            d.insert_value("xesam:album", &t.album.to_variant());
            d.insert_value("mpris:length", &self.duration_us().to_variant());
            if let Some(c) = &t.cover {
                d.insert_value("mpris:artUrl", &gio::File::for_path(c).uri().to_string().to_variant());
            }
        }
        d.end()
    }

    fn status(&self) -> &'static str {
        match self.media.borrow().as_ref() {
            Some(m) if m.is_playing() => "Playing",
            Some(_) => "Paused",
            None => "Stopped",
        }
    }

    fn property(&self, iface: &str, name: &str) -> glib::Variant {
        match (iface, name) {
            (_, "CanQuit") | (_, "HasTrackList") => false.to_variant(),
            (_, "CanRaise") => true.to_variant(),
            (_, "Identity") => tr("Music").to_variant(),
            (_, "DesktopEntry") => "os.kalango.Musica".to_variant(),
            (_, "SupportedUriSchemes") => vec!["file"].to_variant(),
            (_, "SupportedMimeTypes") => vec!["audio/mpeg", "audio/flac", "audio/ogg", "audio/mp4", "audio/x-wav"].to_variant(),
            (_, "PlaybackStatus") => self.status().to_variant(),
            (_, "Metadata") => self.metadata(),
            (_, "Position") => self.media.borrow().as_ref().map(|m| m.timestamp()).unwrap_or(0).to_variant(),
            (_, "Volume") | (_, "Rate") | (_, "MinimumRate") | (_, "MaximumRate") => 1.0f64.to_variant(),
            (_, "CanGoNext") => (self.pos.get() + 1 < self.queue.borrow().len()).to_variant(),
            (_, "CanGoPrevious") | (_, "CanPlay") | (_, "CanPause") | (_, "CanSeek") | (_, "CanControl") => true.to_variant(),
            _ => "".to_variant(),
        }
    }

    fn serve_mpris(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        gio::bus_own_name(gio::BusType::Session, "org.mpris.MediaPlayer2.kalango_musica", gio::BusNameOwnerFlags::NONE, move |c, _| {
            let Some(p) = w.upgrade() else { return };
            *p.mpris.borrow_mut() = Some(c.clone());
            let Ok(info) = gio::DBusNodeInfo::for_xml(MPRIS_XML) else { return };
            for iface_name in ["org.mpris.MediaPlayer2", "org.mpris.MediaPlayer2.Player"] {
                let Some(iface) = info.lookup_interface(iface_name) else { continue };
                let (w1, w2) = (Rc::downgrade(&p), Rc::downgrade(&p));
                let r = c.register_object("/org/mpris/MediaPlayer2", &iface)
                    .method_call(move |_, _, _, _, method, params, inv| {
                        let Some(p) = w1.upgrade() else { return };
                        match method {
                            "Raise" => p.window.present(),
                            "PlayPause" => p.toggle(),
                            "Play" => {
                                if !p.playing() {
                                    p.toggle();
                                }
                            }
                            "Pause" | "Stop" => {
                                if p.playing() {
                                    p.toggle();
                                }
                            }
                            "Next" => p.next(),
                            "Previous" => p.previous(),
                            "Seek" => p.seek_by(params.child_value(0).get::<i64>().unwrap_or(0) as f64 / 1_000_000.0),
                            "SetPosition" => {
                                if let Some(m) = p.media.borrow().as_ref() {
                                    m.seek(params.child_value(1).get::<i64>().unwrap_or(0));
                                }
                            }
                            "OpenUri" => {
                                if let Some(f) = params.child_value(0).get::<String>().and_then(|u| gio::File::for_uri(&u).path()) {
                                    p.play_file(&f);
                                }
                            }
                            _ => {}
                        }
                        inv.return_value(None);
                    })
                    .property(move |_, _, _, iface, name| w2.upgrade().map(|p| p.property(iface, name)).unwrap_or_else(|| "".to_variant()))
                    .build();
                if let Err(e) = r {
                    eprintln!("kalango-shell: música: MPRIS: {e}");
                }
            }
            if debug() {
                eprintln!("debug musica: MPRIS no ar (org.mpris.MediaPlayer2.kalango_musica)");
            }
        }, |_, _| {}, |_, _| {});
    }

    /// Tells MPRIS listeners (the island, the widget) what changed.
    fn mpris_changed(&self) {
        let Some(c) = self.mpris.borrow().clone() else { return };
        let d = glib::VariantDict::new(None);
        for name in ["PlaybackStatus", "Metadata", "CanGoNext"] {
            d.insert_value(name, &self.property("org.mpris.MediaPlayer2.Player", name));
        }
        // (built item by item: a Variant inside a tuple's to_variant() would go as "v", not as the a{sv} the signal is)
        let args = glib::Variant::tuple_from_iter(["org.mpris.MediaPlayer2.Player".to_variant(), d.end(), Vec::<String>::new().to_variant()]);
        let _ = c.emit_signal(None, "/org/mpris/MediaPlayer2", "org.freedesktop.DBus.Properties", "PropertiesChanged", Some(&args));
    }
}
