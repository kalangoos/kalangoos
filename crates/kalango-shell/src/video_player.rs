//! KalangoOS's own Video app (design "Reprodutor de vídeo", 58975; app id os.kalango.Video; `kalango-shell video
//! [FILE]`): the video fills the window, its controls float over it and fade 2 s after the pointer stops (180 ms),
//! back when it moves. The timeline's marks are the chapters the agent found: the scene changes between frames, each
//! named by the local vision model from its frames. "Legendas ao vivo" transcribes the sound a few seconds ahead with
//! the local audio model; "Resumir o vídeo" and the request field ("pule para a parte do pôr do sol") use the
//! chapters and what is said. Played with GStreamer (speed with the pitch kept, volume); muted in a test profile.

use crate::conversa::{set_icon, ui_icon};
use crate::superficies::{caixa, rotulo};
use crate::video_media as media;
use crate::Shell;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gtk::prelude::*;
use gtk::{cairo, gdk, gio, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};
use std::time::Duration;

extern "C" {
    fn gdk_wayland_toplevel_set_application_id(toplevel: *mut gdk::ffi::GdkToplevel, id: *const std::ffi::c_char);
}

pub const VIDEO: &[&str] = &["mp4", "m4v", "mkv", "webm", "mov", "avi", "ogv", "mpg", "mpeg", "wmv", "flv", "3gp", "ts"];
/// Seconds of sound per live caption.
const CAPTION: usize = 6;
/// Seconds of sound per piece heard for the summary.
const HEARD: usize = 30;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

fn muted_by_test() -> bool {
    std::env::var_os("KALANGO_AREA_TESTE").is_some() || std::env::var("KALANGO_SONS").as_deref() == Ok("nao")
}

fn clock(t: f64) -> String {
    let t = t.max(0.0) as u64;
    if t >= 3600 {
        format!("{}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
    } else {
        format!("{:02}:{:02}", t / 60, t % 60)
    }
}

pub fn is_video(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| VIDEO.contains(&e.to_lowercase().as_str()))
}

fn folder_videos(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_file() && is_video(p)).collect();
    v.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    v
}

type SoundWaiter = Box<dyn FnOnce(&Rc<VideoPlayer>, Arc<Vec<i16>>)>;

/// A decoded frame waiting for the screen.
struct Raw {
    width: u32,
    height: u32,
    stride: usize,
    data: Vec<u8>,
}

pub struct VideoPlayer {
    window: gtk::ApplicationWindow,
    toolbar: gtk::WindowHandle,
    line: gtk::Separator,
    name: gtk::Label,
    picture: gtk::Picture,
    stage: gtk::Overlay,
    controls: gtk::Box,
    timeline: gtk::DrawingArea,
    play: gtk::Button,
    prev: gtk::Button,
    next: gtk::Button,
    time: gtk::Label,
    summary_b: gtk::Button,
    captions_b: gtk::Button,
    volume_b: gtk::MenuButton,
    volume_s: gtk::Scale,
    speed_b: gtk::MenuButton,
    mini_b: gtk::Button,
    full_b: gtk::Button,
    caption: gtk::Label,
    card: gtk::Box,
    card_title: gtk::Label,
    card_text: gtk::Label,
    ask: gtk::Entry,
    empty: gtk::Label,
    menu_items: gtk::Box,
    menu_pop: gtk::Popover,
    pipeline: RefCell<Option<gst::Element>>,
    bus_watch: RefCell<Option<gst::bus::BusWatchGuard>>,
    frame: Arc<Mutex<Option<Raw>>>,
    path: RefCell<Option<PathBuf>>,
    length: Cell<f64>,
    position: Cell<f64>,
    rate: Cell<f64>,
    playing: Cell<bool>,
    ended: Cell<bool>,
    volume: Cell<f64>,
    muted: Cell<bool>,
    dragging: Cell<bool>,
    chapters: RefCell<Vec<(f64, String)>>,
    /// where to go once the video is open and its length known ("Ir para" from the conversation)
    start_at: Cell<Option<f64>>,
    captions_on: Cell<bool>,
    sound: RefCell<Option<Arc<Vec<i16>>>>,
    sound_loading: Cell<bool>,
    said: RefCell<BTreeMap<usize, String>>,
    transcribing: Cell<bool>,
    heard: RefCell<Vec<(f64, String)>>,
    last_motion: Cell<i64>,
    over_controls: Cell<bool>,
    compact: Cell<bool>,
    clicks: Cell<i32>,
    pointer: Cell<(f64, f64)>,
    announced: Cell<bool>,
    sound_waiters: RefCell<Vec<SoundWaiter>>,
    generation: Cell<u64>,
    /// the position timer runs (only while the window shows or the video plays)
    ticking: Cell<bool>,
    shell: Weak<Shell>,
}

thread_local! {
    static PLAYER: RefCell<Option<Rc<VideoPlayer>>> = const { RefCell::new(None) };
}

/// Opens the app, with a video when one is given.
pub fn open(shell: &Rc<Shell>, file: Option<PathBuf>) {
    let p = PLAYER.with(|p| p.borrow().clone()).unwrap_or_else(|| {
        let app = gio::Application::default().and_downcast::<gtk::Application>().expect("app");
        let p = VideoPlayer::new(&app, shell);
        PLAYER.with(|x| *x.borrow_mut() = Some(p.clone()));
        p
    });
    if !p.window.is_visible() {
        let (w, h) = crate::superficies::tamanho_de_janela(1500, 910);
        p.window.set_default_size(w, h);
    }
    p.window.present();
    if let Some(f) = file {
        p.load(&f);
    } else if p.path.borrow().is_none() {
        p.empty.set_visible(true);
    }
}

/// The chapters of a video, found now (off the interface's thread): frames taken across it, the scene changes between
/// them, each scene named by the local vision model from its middle frame. The Video app and the conversation's video
/// card use it.
pub fn find_chapters_now(path: &Path, bin: &Path) -> Result<Vec<(f64, String)>, String> {
    let (_, len) = media::frames(path, &[], 8, 8)?;
    if len < 8.0 {
        return Ok(Vec::new());
    }
    let step = (len / 120.0).clamp(1.0, 10.0);
    let times: Vec<f64> = (0..).map(|i| f64::from(i) * step).take_while(|t| *t < len).collect();
    let (fs, _) = media::frames(path, &times, 64, 36)?;
    let mut starts = vec![0.0];
    // (each change found between two frames a step apart, then its exact time)
    starts.extend(media::scene_changes(&fs, (len / 20.0).max(4.0), 9).into_iter().map(|t| media::refine_change(path, (t - step).max(0.0), t)));
    let mids: Vec<f64> = starts.iter().enumerate().map(|(i, s)| (s + starts.get(i + 1).copied().unwrap_or(len)) / 2.0).collect();
    let (named, _) = media::frames(path, &mids, 320, 180)?;
    let tmp = std::env::temp_dir().join(format!("kalango-video-{}.png", std::process::id()));
    let pt = kalango_i18n::system_locale().is_none_or(|l| l.starts_with("pt"));
    let question = if pt { "Diga em no máximo 5 palavras o que aparece nesta cena de vídeo. Só as palavras, sem ponto final." }
        else { "Say in at most 5 words what this video scene shows. Just the words, no full stop." };
    let mut out = Vec::new();
    for (s, f) in starts.iter().zip(named.iter()) {
        let mut title = String::new();
        if media::png(f, &tmp).is_ok() {
            // (the model's answer only — an error is no name; a model starting up gets one more try)
            for attempt in 0..2 {
                let out = std::process::Command::new(bin).args(["--json", "ver"]).arg(&tmp).arg(question).stderr(std::process::Stdio::null()).output();
                let v = out.ok().and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok()).unwrap_or_default();
                if v["tipo"] == "visao" {
                    title = v["texto"].as_str().unwrap_or("").to_string();
                    break;
                }
                if attempt == 0 {
                    std::thread::sleep(Duration::from_secs(3));
                }
            }
        }
        let title = title.trim().trim_end_matches('.').lines().next().unwrap_or("").chars().take(48).collect::<String>();
        out.push((*s, title));
    }
    let _ = std::fs::remove_file(&tmp);
    Ok(out)
}

/// The chapters found before (kept by the Video app or the conversation), none when not yet.
pub fn cached_chapters(path: &Path) -> Vec<(f64, String)> {
    let v = std::fs::read_to_string(media::cache_file(path)).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()).unwrap_or_default();
    v["capitulos"].as_array().into_iter().flatten().map(|c| (c["t"].as_f64().unwrap_or(0.0), c["titulo"].as_str().unwrap_or("").to_string())).collect()
}

/// Keeps a video's chapters (what was heard in it stays).
pub fn save_chapters(path: &Path, ch: &[(f64, String)]) {
    let f = media::cache_file(path);
    let mut v = std::fs::read_to_string(&f).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()).filter(|v| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}));
    v["capitulos"] = serde_json::json!(ch.iter().map(|(t, n)| serde_json::json!({"t": t, "titulo": n})).collect::<Vec<_>>());
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, v.to_string());
}

/// Opens the app with a video, from `at` seconds ("Ir para 07:15" in the conversation).
pub fn open_at(shell: &Rc<Shell>, file: PathBuf, at: f64) {
    open(shell, Some(file));
    if let Some(p) = PLAYER.with(|p| p.borrow().clone()) {
        p.start_at.set(Some(at));
    }
}

fn button(glyph: &str, color: &str, px: i32, class: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class(class);
    b.set_child(Some(&ui_icon(glyph, color, px)));
    b.set_tooltip_text(Some(tip));
    b.set_valign(Align::Center);
    b
}

/// The glyph inside a button (a button's image, a menu button's, or the first of a button with words).
fn glyph_of(b: &impl IsA<gtk::Widget>) -> Option<gtk::Image> {
    let w = b.as_ref();
    let first = w.first_child()?;
    first.clone().downcast::<gtk::Image>().ok().or_else(|| first.first_child().and_then(|c| c.downcast::<gtk::Image>().ok()))
}

fn text_button(glyph: &str, color: &str, label: &str, class: &str) -> (gtk::Button, gtk::Image, gtk::Label) {
    let b = gtk::Button::new();
    b.add_css_class(class);
    let r = caixa(Orientation::Horizontal, 6, "");
    let i = ui_icon(glyph, color, 14);
    let l = rotulo(label, "");
    r.append(&i);
    r.append(&l);
    b.set_child(Some(&r));
    b.set_valign(Align::Center);
    (b, i, l)
}

impl VideoPlayer {
    fn new(app: &gtk::Application, shell: &Rc<Shell>) -> Rc<VideoPlayer> {
        media::init();
        let (w, h) = crate::superficies::tamanho_de_janela(1500, 910);
        let window = gtk::ApplicationWindow::builder().application(app).title(tr("Video")).default_width(w).default_height(h).build();
        // (its app id as its name: the state written by `kalango-shell estado` finds it by it)
        window.set_widget_name("os.kalango.Video");
        // (one window for the app, kept: closing hides it and opening shows it again — a closed GTK window shown again
        // could not be closed any more, and it kept text input from the other windows)
        window.set_hide_on_close(true);
        // (closed for a while, the window lets go of its surface and renderer and keeps its widgets: see
        // superficies::release_resources_when_hidden — not while it plays: its frames keep coming into the window's picture)
        crate::superficies::release_resources_when_hidden(&window, || {
            crate::release_policy::video(PLAYER.with(|x| x.borrow().as_ref().is_some_and(|p| p.playing.get())))
        });
        window.add_css_class("ed-janela");
        let sem_barra = gtk::Box::new(Orientation::Horizontal, 0);
        sem_barra.set_visible(false);
        window.set_titlebar(Some(&sem_barra));

        // the bar: the app, the file, the request to the agent, the menu, the window's buttons
        let bar = caixa(Orientation::Horizontal, 10, "ed-barra");
        bar.append(&crate::apps::imagem_kalango("video", 24));
        let name = rotulo(tr("Video"), "img-titulo");
        name.set_hexpand(true);
        name.set_xalign(0.0);
        name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        name.set_max_width_chars(1);
        bar.append(&name);
        let ask = gtk::Entry::new();
        ask.add_css_class("ed-pedir");
        ask.set_size_request(340, 32);
        ask.set_valign(Align::Center);
        ask.set_placeholder_text(Some(tr("Ask: “jump to the sunset part”")));
        ask.set_primary_icon_paintable(ui_icon("spark", "#A5A1E2", 13).paintable().as_ref());
        let ab = gtk::Overlay::new();
        ab.set_child(Some(&ask));
        let key = rotulo("Ctrl K", "tb-atalho");
        key.set_halign(Align::End);
        key.set_valign(Align::Center);
        key.set_margin_end(8);
        key.set_can_target(false);
        ab.add_overlay(&key);
        bar.append(&ab);
        let menu = gtk::MenuButton::new();
        menu.add_css_class("calc-menu");
        menu.set_child(Some(&ui_icon("menu", "#8E8F99", 16)));
        menu.set_valign(Align::Center);
        let menu_pop = gtk::Popover::new();
        menu_pop.add_css_class("menu");
        let menu_items = caixa(Orientation::Vertical, 2, "menu-lista");
        menu_pop.set_child(Some(&menu_items));
        menu.set_popover(Some(&menu_pop));
        bar.append(&menu);
        bar.append(&crate::configuracoes::controles_janela());
        let toolbar = gtk::WindowHandle::new();
        toolbar.set_child(Some(&bar));
        let line = gtk::Separator::new(Orientation::Horizontal);
        line.add_css_class("vid-linha");

        // the stage: the picture, and over it the caption, the controls, the agent's card
        let picture = gtk::Picture::new();
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        let back = caixa(Orientation::Vertical, 0, "vid-palco");
        back.append(&picture);
        let stage = gtk::Overlay::new();
        stage.set_child(Some(&back));
        stage.set_vexpand(true);
        let caption = rotulo("", "vid-legenda");
        caption.set_wrap(true);
        caption.set_max_width_chars(60);
        caption.set_justify(gtk::Justification::Center);
        caption.set_halign(Align::Center);
        caption.set_valign(Align::End);
        caption.set_margin_bottom(136);
        caption.set_visible(false);
        caption.set_can_target(false);
        stage.add_overlay(&caption);

        let controls = caixa(Orientation::Vertical, 10, "vid-controles");
        controls.set_valign(Align::End);
        controls.set_margin_start(24);
        controls.set_margin_end(24);
        controls.set_margin_bottom(20);
        let timeline = gtk::DrawingArea::new();
        timeline.set_content_height(14);
        timeline.set_hexpand(true);
        timeline.set_cursor_from_name(Some("pointer"));
        timeline.set_has_tooltip(true);
        crate::central::posicao_debug_sempre("video linha do tempo", timeline.upcast_ref());
        controls.append(&timeline);
        let row = caixa(Orientation::Horizontal, 12, "");
        let prev = button("anterior", "#E2E2E6", 12, "vid-botao", tr("Previous video"));
        let play = button("tocar", "#1C1D24", 12, "vid-tocar", tr("Play"));
        let next = button("proxima", "#E2E2E6", 12, "vid-botao", tr("Next video"));
        let time = rotulo("00:00 / 00:00", "vid-tempo");
        time.set_margin_start(2);
        for (b, n) in [(&prev, "anterior"), (&play, "tocar"), (&next, "proxima")] {
            crate::central::posicao_debug_sempre(&format!("video {n}"), b.upcast_ref());
            row.append(b);
        }
        row.append(&time);
        let space = caixa(Orientation::Horizontal, 0, "");
        space.set_hexpand(true);
        row.append(&space);
        let (summary_b, _, _) = text_button("spark", "#A5A1E2", tr("Summarize the video"), "vid-acao");
        let (captions_b, _, _) = text_button("doc", "#E2E2E6", tr("Live captions"), "vid-acao");
        crate::central::posicao_debug_sempre("video resumir", summary_b.upcast_ref());
        crate::central::posicao_debug_sempre("video legendas", captions_b.upcast_ref());
        row.append(&summary_b);
        row.append(&captions_b);
        // the volume (its slider) and the speed
        let volume_b = gtk::MenuButton::new();
        volume_b.add_css_class("vid-botao");
        volume_b.set_child(Some(&ui_icon("volume", "#E2E2E6", 14)));
        volume_b.set_tooltip_text(Some(tr("Volume")));
        volume_b.set_valign(Align::Center);
        let vp = gtk::Popover::new();
        vp.add_css_class("menu");
        let volume_s = gtk::Scale::with_range(Orientation::Vertical, 0.0, 1.0, 0.05);
        volume_s.set_inverted(true);
        volume_s.set_size_request(-1, 120);
        volume_s.set_value(1.0);
        volume_s.add_css_class("vid-volume");
        vp.set_child(Some(&volume_s));
        volume_b.set_popover(Some(&vp));
        row.append(&volume_b);
        let speed_b = gtk::MenuButton::new();
        speed_b.add_css_class("vid-acao");
        speed_b.set_child(Some(&rotulo("1×", "")));
        speed_b.set_tooltip_text(Some(tr("Speed")));
        speed_b.set_valign(Align::Center);
        crate::central::posicao_debug_sempre("video velocidade", speed_b.upcast_ref());
        row.append(&speed_b);
        let mini_b = button("layout", "#E2E2E6", 14, "vid-botao", tr("Mini player"));
        let full_b = button("maximizar", "#E2E2E6", 12, "vid-botao", tr("Full screen"));
        crate::central::posicao_debug_sempre("video mini", mini_b.upcast_ref());
        crate::central::posicao_debug_sempre("video tela cheia", full_b.upcast_ref());
        row.append(&mini_b);
        row.append(&full_b);
        controls.append(&row);
        stage.add_overlay(&controls);

        // the agent's card (a summary, an answer), at the top right
        let card = caixa(Orientation::Vertical, 6, "vid-cartao");
        card.set_halign(Align::End);
        card.set_valign(Align::Start);
        card.set_margin_top(20);
        card.set_margin_end(24);
        card.set_size_request(380, -1);
        let ch = caixa(Orientation::Horizontal, 8, "");
        ch.append(&ui_icon("spark", "#A5A1E2", 14));
        let card_title = rotulo("", "vid-cartao-titulo");
        card_title.set_hexpand(true);
        card_title.set_xalign(0.0);
        ch.append(&card_title);
        let close = button("fechar", "#8E8F99", 10, "vid-botao", tr("Close"));
        ch.append(&close);
        card.append(&ch);
        let card_text = rotulo("", "vid-cartao-texto");
        card_text.set_wrap(true);
        card_text.set_xalign(0.0);
        card_text.set_max_width_chars(1);
        card_text.set_selectable(true);
        card.append(&card_text);
        card.set_visible(false);
        stage.add_overlay(&card);
        let empty = rotulo(tr("Open a video with the menu, or from the Files app."), "img-vazio");
        empty.set_visible(false);
        empty.set_halign(Align::Center);
        empty.set_valign(Align::Center);
        stage.add_overlay(&empty);

        let root = caixa(Orientation::Vertical, 0, "");
        root.append(&toolbar);
        root.append(&line);
        root.append(&stage);
        window.set_child(Some(&root));

        let p = Rc::new(VideoPlayer {
            window, toolbar, line, name, picture, stage, controls, timeline, play, prev, next, time, summary_b, captions_b, volume_b, volume_s, speed_b,
            mini_b, full_b, caption, card, card_title, card_text, ask, empty, menu_items, menu_pop,
            pipeline: RefCell::default(), bus_watch: RefCell::default(), frame: Arc::default(), path: RefCell::default(), length: Cell::new(0.0),
            position: Cell::new(0.0), rate: Cell::new(1.0), playing: Cell::new(false), ended: Cell::new(false), volume: Cell::new(1.0),
            muted: Cell::new(false), dragging: Cell::new(false), chapters: RefCell::default(), start_at: Cell::new(None), captions_on: Cell::new(false), sound: RefCell::default(),
            sound_loading: Cell::new(false), said: RefCell::default(), transcribing: Cell::new(false), heard: RefCell::default(),
            last_motion: Cell::new(glib::monotonic_time()), over_controls: Cell::new(false), compact: Cell::new(false), clicks: Cell::new(0), pointer: Cell::new((-1.0, -1.0)), announced: Cell::new(false),
            sound_waiters: RefCell::default(), generation: Cell::new(0), ticking: Cell::new(false),
            shell: Rc::downgrade(shell),
        });
        p.wire(&close, &vp);
        p.fill_menu();
        p
    }

    fn wire(self: &Rc<Self>, close: &gtk::Button, vp: &gtk::Popover) {
        // a new frame from the stream is shown at the next screen refresh
        let slot = self.frame.clone();
        let w = Rc::downgrade(self);
        self.picture.add_tick_callback(move |pic, _| {
            if let Some(f) = slot.lock().ok().and_then(|mut s| s.take()) {
                let bytes = glib::Bytes::from_owned(f.data);
                let tex = gdk::MemoryTexture::new(f.width as i32, f.height as i32, gdk::MemoryFormat::R8g8b8a8, &bytes, f.stride);
                pic.set_paintable(Some(&tex));
                if let Some(p) = w.upgrade() {
                    p.announce();
                }
            }
            glib::ControlFlow::Continue
        });
        let w = Rc::downgrade(self);
        self.timeline.set_draw_func(move |_, cr, width, height| {
            if let Some(p) = w.upgrade() {
                p.draw_timeline(cr, f64::from(width), f64::from(height));
            }
        });
        // the timeline: a click or a drag goes there; resting on it says the time and the chapter
        let drag = gtk::GestureDrag::new();
        let w = Rc::downgrade(self);
        drag.connect_drag_begin(move |_, x, _| {
            if let Some(p) = w.upgrade() {
                p.dragging.set(true);
                p.seek_x(x, false);
            }
        });
        let w = Rc::downgrade(self);
        drag.connect_drag_update(move |g, dx, _| {
            if let (Some(p), Some((x, _))) = (w.upgrade(), g.start_point()) {
                p.seek_x(x + dx, false);
            }
        });
        let w = Rc::downgrade(self);
        drag.connect_drag_end(move |g, dx, _| {
            if let (Some(p), Some((x, _))) = (w.upgrade(), g.start_point()) {
                p.seek_x(x + dx, true);
                p.dragging.set(false);
            }
        });
        self.timeline.add_controller(drag);
        let w = Rc::downgrade(self);
        self.timeline.connect_query_tooltip(move |tl, x, _, _, tip| {
            let Some(p) = w.upgrade() else { return false };
            let len = p.length.get();
            if len <= 0.0 {
                return false;
            }
            let t = (f64::from(x) / f64::from(tl.width().max(1))).clamp(0.0, 1.0) * len;
            let ch = p.chapters.borrow().iter().rev().find(|(s, _)| *s <= t).map(|(_, n)| n.clone()).filter(|n| !n.is_empty());
            tip.set_text(Some(&match ch { Some(n) => format!("{} · {n}", clock(t)), None => clock(t) }));
            true
        });
        for (b, f) in [
            (&self.play, Box::new(|p: &Rc<VideoPlayer>| p.toggle()) as Box<dyn Fn(&Rc<VideoPlayer>)>),
            (&self.prev, Box::new(|p: &Rc<VideoPlayer>| p.neighbour(-1))),
            (&self.next, Box::new(|p: &Rc<VideoPlayer>| p.neighbour(1))),
            (&self.summary_b, Box::new(|p: &Rc<VideoPlayer>| p.summarize())),
            (&self.captions_b, Box::new(|p: &Rc<VideoPlayer>| p.set_captions(!p.captions_on.get()))),
            (&self.mini_b, Box::new(|p: &Rc<VideoPlayer>| p.set_compact(!p.compact.get()))),
            (&self.full_b, Box::new(|p: &Rc<VideoPlayer>| p.toggle_fullscreen())),
            (close, Box::new(|p: &Rc<VideoPlayer>| p.card.set_visible(false))),
        ] {
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    f(&p);
                }
            });
        }
        // the volume's slider
        let w = Rc::downgrade(self);
        self.volume_s.connect_value_changed(move |s| {
            if let Some(p) = w.upgrade() {
                p.set_volume(s.value(), false);
            }
        });
        let mute = gtk::Button::with_label(tr("Mute"));
        mute.add_css_class("menu-item");
        let w = Rc::downgrade(self);
        mute.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.set_volume(p.volume.get(), !p.muted.get());
            }
        });
        if let Some(c) = vp.child().and_downcast::<gtk::Scale>() {
            vp.set_child(None::<&gtk::Widget>);
            let col = caixa(Orientation::Vertical, 6, "menu-lista");
            c.set_halign(Align::Center);
            col.append(&c);
            col.append(&mute);
            vp.set_child(Some(&col));
        }
        // the speed: its own menu
        let sp = gtk::Popover::new();
        sp.add_css_class("menu");
        let items = caixa(Orientation::Vertical, 2, "menu-lista");
        for r in [0.5, 0.75, 1.0, 1.25, 1.5, 2.0] {
            let b = gtk::Button::with_label(&format!("{}×", r.to_string().replace('.', ",")));
            b.add_css_class("menu-item");
            crate::central::posicao_debug_sempre(&format!("video velocidade {r}"), b.upcast_ref());
            let (w, sp2) = (Rc::downgrade(self), sp.clone());
            b.connect_clicked(move |_| {
                sp2.popdown();
                if let Some(p) = w.upgrade() {
                    p.set_rate(r);
                }
            });
            items.append(&b);
        }
        sp.set_child(Some(&items));
        self.speed_b.set_popover(Some(&sp));
        // a click on the picture plays or pauses; a double click fills the screen
        let click = gtk::GestureClick::new();
        let w = Rc::downgrade(self);
        click.connect_released(move |_, n, _, _| {
            let Some(p) = w.upgrade() else { return };
            p.clicks.set(n);
            if n == 2 {
                p.toggle_fullscreen();
            } else if n == 1 && p.path.borrow().is_some() {
                // (a single click waits a moment: the start of a double one does not pause)
                let w2 = Rc::downgrade(&p);
                glib::timeout_add_local_once(Duration::from_millis(250), move || {
                    if let Some(p) = w2.upgrade() {
                        if p.clicks.get() == 1 {
                            p.toggle();
                        }
                    }
                });
            }
        });
        self.picture.add_controller(click);
        // the pointer: the controls and the cursor come back when it moves, and go 2 s after it stops
        let motion = gtk::EventControllerMotion::new();
        let w = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            // (only a pointer that really moved: each new frame makes GTK say the pointer is there again)
            if let Some(p) = w.upgrade() {
                let (ox, oy) = p.pointer.replace((x, y));
                if (x - ox).abs() + (y - oy).abs() > 1.0 {
                    p.wake();
                }
            }
        });
        self.stage.add_controller(motion);
        let over = gtk::EventControllerMotion::new();
        let w = Rc::downgrade(self);
        over.connect_enter(move |_, _, _| {
            if let Some(p) = w.upgrade() {
                p.over_controls.set(true);
            }
        });
        let w = Rc::downgrade(self);
        over.connect_leave(move |_| {
            if let Some(p) = w.upgrade() {
                p.over_controls.set(false);
            }
        });
        self.controls.add_controller(over);
        // the request: a time to jump to, or an answer in the card
        let w = Rc::downgrade(self);
        self.ask.connect_activate(move |e| {
            let t = e.text().trim().to_string();
            if let (Some(p), false) = (w.upgrade(), t.is_empty()) {
                e.set_text("");
                // (the keys go back to the player)
                gtk::prelude::GtkWindowExt::set_focus(&p.window, None::<&gtk::Widget>);
                p.request(t);
            }
        });
        // keys
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, k, _, m| {
            let Some(p) = w.upgrade() else { return glib::Propagation::Proceed };
            let ctrl = m.contains(gdk::ModifierType::CONTROL_MASK);
            // (typing in the request is typing: its focus is the text inside it)
            let typing = gtk::prelude::GtkWindowExt::focus(&p.window).is_some_and(|f| f.is_ancestor(&p.ask) || f == *p.ask.upcast_ref::<gtk::Widget>());
            if typing && !(ctrl && k == gdk::Key::k) && k != gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            p.wake();
            match k {
                gdk::Key::k if ctrl => {
                    p.ask.grab_focus();
                }
                gdk::Key::Escape if typing => {
                    gtk::prelude::GtkWindowExt::set_focus(&p.window, None::<&gtk::Widget>);
                }
                gdk::Key::Escape => {
                    if p.window.is_fullscreen() {
                        p.window.unfullscreen();
                    } else if p.compact.get() {
                        p.set_compact(false);
                    } else if p.card.is_visible() {
                        p.card.set_visible(false);
                    } else {
                        return glib::Propagation::Proceed;
                    }
                }
                gdk::Key::space | gdk::Key::k => p.toggle(),
                gdk::Key::Right => p.seek_by(5.0),
                gdk::Key::Left => p.seek_by(-5.0),
                gdk::Key::Up => p.set_volume((p.volume.get() + 0.1).min(1.0), false),
                gdk::Key::Down => p.set_volume((p.volume.get() - 0.1).max(0.0), false),
                gdk::Key::f | gdk::Key::F => p.toggle_fullscreen(),
                gdk::Key::m | gdk::Key::M => p.set_volume(p.volume.get(), !p.muted.get()),
                gdk::Key::c | gdk::Key::C => p.set_captions(!p.captions_on.get()),
                gdk::Key::Home => p.seek_to(0.0, true),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.window.add_controller(keys);
        // the full screen: no bar, the button's glyph
        let w = Rc::downgrade(self);
        self.window.connect_fullscreened_notify(move |win| {
            if let Some(p) = w.upgrade() {
                let full = win.is_fullscreen();
                p.toolbar.set_visible(!full && !p.compact.get());
                p.line.set_visible(!full && !p.compact.get());
                p.full_b.set_tooltip_text(Some(if full { tr("Leave full screen") } else { tr("Full screen") }));
                if debug() {
                    eprintln!("debug video: tela cheia {full}");
                }
            }
        });
        // closing the window stops the video
        let w = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(p) = w.upgrade() {
                p.stop();
            }
            glib::Propagation::Proceed
        });
        let app_id = |j: &gtk::ApplicationWindow| {
            if let Some(s) = j.surface().filter(|s| s.type_().name().contains("Wayland")) {
                let id = std::ffi::CString::new("os.kalango.Video").unwrap();
                unsafe { gdk_wayland_toplevel_set_application_id(s.as_ptr() as *mut gdk::ffi::GdkToplevel, id.as_ptr()) };
            }
        };
        self.window.connect_realize(move |j| app_id(j));
        self.window.connect_map(move |j| {
            app_id(j);
            // (the keys are the player's: the request field is focused only when asked, Ctrl+K)
            let j = j.clone();
            glib::idle_add_local_once(move || gtk::prelude::GtkWindowExt::set_focus(&j, None::<&gtk::Widget>));
        });
        // the position, the captions and the controls' fading, five times a second — while the window shows or the
        // video plays
        self.ensure_ticking();
        let w = Rc::downgrade(self);
        // (shown again: brought up to date at once — paused or moved by MPRIS or a media key while the timer slept —,
        // as the timer that never slept showed it)
        self.window.connect_map(move |_| {
            if let Some(p) = w.upgrade() {
                p.tick();
                p.ensure_ticking();
            }
        });
    }

    fn ensure_ticking(self: &Rc<Self>) {
        if self.ticking.replace(true) {
            return;
        }
        let w = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(200), move || {
            let Some(p) = w.upgrade() else { return glib::ControlFlow::Break };
            if !crate::release_policy::keep_ticking(p.window.is_visible(), p.playing.get()) {
                // (a last time: what stopped it — the pause — shows when the window comes back)
                p.tick();
                p.ticking.set(false);
                return glib::ControlFlow::Break;
            }
            p.tick();
            glib::ControlFlow::Continue
        });
    }

    fn fill_menu(self: &Rc<Self>) {
        while let Some(c) = self.menu_items.first_child() {
            self.menu_items.remove(&c);
        }
        let add = |label: &str, f: Box<dyn Fn(&Rc<VideoPlayer>)>| {
            let b = gtk::Button::with_label(label);
            b.add_css_class("menu-item");
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            let (w, pop) = (Rc::downgrade(self), self.menu_pop.clone());
            b.connect_clicked(move |_| {
                pop.popdown();
                if let Some(p) = w.upgrade() {
                    f(&p);
                }
            });
            self.menu_items.append(&b);
        };
        add(tr("Open…"), Box::new(|p| p.choose()));
        add(tr("Show in the folder"), Box::new(|p| {
            if let (Some(f), Some(s)) = (p.path.borrow().clone(), p.shell.upgrade()) {
                s.abrir_arquivos(Some(&f));
            }
        }));
        // the chapters the agent found: a click goes there
        let chapters = self.chapters.borrow().clone();
        if chapters.len() > 1 {
            let t = rotulo(tr("CHAPTERS"), "menu-titulo");
            t.set_xalign(0.0);
            self.menu_items.append(&t);
            for (s, n) in chapters {
                add(&format!("{}  {}", clock(s), if n.is_empty() { tr("Chapter") } else { &n }), Box::new(move |p| p.seek_to(s, true)));
            }
        }
    }

    fn choose(self: &Rc<Self>) {
        let d = gtk::FileDialog::new();
        d.set_title(tr("Open a video"));
        let f = gtk::FileFilter::new();
        f.set_name(Some(tr("Videos")));
        f.add_mime_type("video/*");
        let fs = gio::ListStore::new::<gtk::FileFilter>();
        fs.append(&f);
        d.set_filters(Some(&fs));
        let w = Rc::downgrade(self);
        d.open(Some(&self.window), gio::Cancellable::NONE, move |r| {
            if let (Some(p), Ok(f)) = (w.upgrade(), r) {
                if let Some(path) = f.path() {
                    p.load(&path);
                }
            }
        });
    }

    // ------------------------------------------------------------------------------------------- playing

    fn stop(&self) {
        if let Some(pb) = self.pipeline.borrow_mut().take() {
            let _ = pb.set_state(gst::State::Null);
        }
        self.bus_watch.borrow_mut().take();
        self.playing.set(false);
    }

    /// Opens a video: plays it from the start, and the agent starts finding its chapters.
    fn load(self: &Rc<Self>, path: &Path) {
        self.stop();
        // (a frame of the video before, still waiting: not this one's)
        if let Ok(mut f) = self.frame.lock() {
            *f = None;
        }
        self.generation.set(self.generation.get() + 1);
        *self.path.borrow_mut() = Some(path.to_path_buf());
        self.chapters.borrow_mut().clear();
        self.said.borrow_mut().clear();
        self.heard.borrow_mut().clear();
        self.sound.borrow_mut().take();
        self.sound_loading.set(false);
        self.transcribing.set(false);
        self.caption.set_visible(false);
        self.card.set_visible(false);
        self.empty.set_visible(false);
        self.length.set(0.0);
        self.position.set(0.0);
        self.ended.set(false);
        self.announced.set(false);
        self.picture.set_paintable(None::<&gdk::Paintable>);
        let file_name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.name.set_text(&file_name);
        self.window.set_title(Some(&file_name));
        let list = path.parent().map(folder_videos).unwrap_or_default();
        let i = list.iter().position(|x| x == path);
        self.prev.set_sensitive(i.is_some_and(|i| i > 0));
        self.next.set_sensitive(i.is_some_and(|i| i + 1 < list.len()));
        let r = self.build_pipeline(path);
        if let Err(e) = r {
            self.say(tr("This video could not be played"), &e);
            eprintln!("kalango-shell: vídeo: {e}");
            return;
        }
        self.fill_menu();
        self.find_chapters(path);
        // (what was learnt of it before: its chapters and words)
        self.read_cache(path);
        self.wake();
    }

    fn build_pipeline(self: &Rc<Self>, path: &Path) -> Result<(), String> {
        let pb = gst::ElementFactory::make("playbin").build().map_err(|e| e.to_string())?;
        let sinkbin = gst::parse::bin_from_description("videoconvert ! appsink name=s caps=video/x-raw,format=RGBA max-buffers=2 drop=true", true)
            .map_err(|e| e.to_string())?;
        let sink = sinkbin.by_name("s").and_downcast::<gst_app::AppSink>().ok_or("appsink")?;
        let slot = self.frame.clone();
        sink.set_callbacks(gst_app::AppSinkCallbacks::builder().new_sample(move |s| {
            let sample = s.pull_sample().map_err(|_| gst::FlowError::Eos)?;
            let info = sample.caps().and_then(|c| gstreamer_video::VideoInfo::from_caps(c).ok()).ok_or(gst::FlowError::Error)?;
            let buf = sample.buffer().ok_or(gst::FlowError::Error)?;
            let map = buf.map_readable().map_err(|_| gst::FlowError::Error)?;
            if let Ok(mut s) = slot.lock() {
                *s = Some(Raw { width: info.width(), height: info.height(), stride: info.stride()[0] as usize, data: map.as_slice().to_vec() });
            }
            Ok(gst::FlowSuccess::Ok)
        }).build());
        pb.set_property("video-sink", &sinkbin);
        // (a faster or slower video keeps its voices' pitch)
        if let Ok(st) = gst::ElementFactory::make("scaletempo").build() {
            pb.set_property("audio-filter", &st);
        }
        pb.set_property("uri", gio::File::for_path(path).uri().to_string());
        pb.set_property("volume", self.volume.get());
        pb.set_property("mute", self.muted.get() || muted_by_test());
        let bus = pb.bus().ok_or("bus")?;
        let w = Rc::downgrade(self);
        let gen = self.generation.get();
        let watch = bus.add_watch_local(move |_, msg| {
            let Some(p) = w.upgrade() else { return glib::ControlFlow::Break };
            if p.generation.get() != gen {
                return glib::ControlFlow::Break;
            }
            match msg.view() {
                gst::MessageView::Eos(_) => {
                    p.playing.set(false);
                    p.ended.set(true);
                    if let Some(pb) = p.pipeline.borrow().as_ref() {
                        let _ = pb.set_state(gst::State::Paused);
                    }
                    p.position.set(p.length.get());
                    p.update_play();
                    p.wake();
                    if debug() {
                        eprintln!("debug video: chegou ao fim");
                    }
                }
                gst::MessageView::Error(e) => {
                    eprintln!("kalango-shell: vídeo: {}", e.error());
                    p.say(tr("This video could not be played"), &e.error().to_string());
                    p.stop();
                }
                gst::MessageView::AsyncDone(_) | gst::MessageView::DurationChanged(_) => p.read_length(),
                _ => {}
            }
            glib::ControlFlow::Continue
        }).map_err(|e| e.to_string())?;
        *self.bus_watch.borrow_mut() = Some(watch);
        pb.set_state(gst::State::Playing).map_err(|e| e.to_string())?;
        *self.pipeline.borrow_mut() = Some(pb);
        self.playing.set(true);
        self.ensure_ticking();
        self.rate.set(1.0);
        self.speed_label("1×");
        self.update_play();
        Ok(())
    }

    fn read_length(&self) {
        let Some(pb) = self.pipeline.borrow().clone() else { return };
        if let Some(d) = pb.query_duration::<gst::ClockTime>() {
            let len = d.nseconds() as f64 / 1e9;
            if (len - self.length.get()).abs() > 0.01 {
                self.length.set(len);
                if let Some(t) = self.start_at.take() {
                    self.seek_to(t, true);
                }
                self.announce();
                self.timeline.queue_draw();
            }
        }
    }

    /// Says (for the session test) that the video is open: once its first frame is in view and its length known.
    fn announce(&self) {
        let (w, h) = self.frame_size();
        if self.announced.get() || w == 0 || self.length.get() <= 0.0 {
            return;
        }
        self.announced.set(true);
        if debug() {
            let path = self.path.borrow().clone().unwrap_or_default();
            eprintln!("debug video: aberto «{}» ({w}×{h}, {} s){}", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                self.length.get().round(), if muted_by_test() { " · mudo (teste)" } else { "" });
        }
    }

    fn frame_size(&self) -> (i32, i32) {
        self.picture.paintable().map(|p| (p.intrinsic_width(), p.intrinsic_height())).unwrap_or((0, 0))
    }

    fn toggle(self: &Rc<Self>) {
        let Some(pb) = self.pipeline.borrow().clone() else {
            if let Some(p) = self.path.borrow().clone() {
                self.load(&p);
            }
            return;
        };
        if self.ended.get() {
            self.ended.set(false);
            self.seek_to(0.0, true);
        }
        let now = !self.playing.get();
        let _ = pb.set_state(if now { gst::State::Playing } else { gst::State::Paused });
        self.playing.set(now);
        if now {
            self.ensure_ticking();
        }
        self.update_play();
        self.wake();
        if debug() {
            eprintln!("debug video: {} em {}s", if now { "tocando" } else { "pausado" }, self.position.get().floor());
        }
    }

    fn update_play(&self) {
        if let Some(i) = glyph_of(&self.play) {
            set_icon(&i, if self.playing.get() { "pausa" } else { "tocar" }, "#1C1D24");
        }
        self.play.set_tooltip_text(Some(if self.playing.get() { tr("Pause") } else { tr("Play") }));
    }

    fn seek_to(&self, t: f64, accurate: bool) {
        let Some(pb) = self.pipeline.borrow().clone() else { return };
        let len = self.length.get();
        let t = if len > 0.0 { t.clamp(0.0, (len - 0.05).max(0.0)) } else { t.max(0.0) };
        let flags = gst::SeekFlags::FLUSH | if accurate { gst::SeekFlags::ACCURATE } else { gst::SeekFlags::KEY_UNIT };
        let at = Some(gst::ClockTime::from_nseconds((t * 1e9) as u64));
        let _ = pb.seek(self.rate.get(), flags, gst::SeekType::Set, at, gst::SeekType::None, gst::ClockTime::NONE);
        self.position.set(t);
        self.ended.set(false);
        self.timeline.queue_draw();
        if debug() && accurate {
            eprintln!("debug video: posição {}s", t.round());
        }
    }

    fn seek_by(&self, s: f64) {
        self.seek_to(self.position.get() + s, true);
    }

    fn seek_x(&self, x: f64, accurate: bool) {
        let w = f64::from(self.timeline.width()).max(1.0);
        let len = self.length.get();
        if len > 0.0 {
            self.seek_to((x / w).clamp(0.0, 1.0) * len, accurate);
        }
    }

    fn set_rate(&self, r: f64) {
        self.rate.set(r);
        self.speed_label(&format!("{}×", r.to_string().replace('.', ",")));
        self.seek_to(self.position.get(), true);
        if debug() {
            eprintln!("debug video: velocidade {r}");
        }
    }

    fn speed_label(&self, t: &str) {
        self.speed_b.set_child(Some(&rotulo(t, "")));
    }

    fn set_volume(&self, v: f64, muted: bool) {
        self.volume.set(v);
        self.muted.set(muted || v <= 0.001);
        if let Some(pb) = self.pipeline.borrow().as_ref() {
            pb.set_property("volume", v);
            pb.set_property("mute", self.muted.get() || muted_by_test());
        }
        if (self.volume_s.value() - v).abs() > 0.001 {
            self.volume_s.set_value(v);
        }
        if let Some(i) = glyph_of(&self.volume_b) {
            set_icon(&i, if self.muted.get() { "volume-mudo" } else if v < 0.5 { "volume-baixo" } else { "volume" }, "#E2E2E6");
        }
    }

    fn neighbour(self: &Rc<Self>, d: i32) {
        let Some(cur) = self.path.borrow().clone() else { return };
        let list = cur.parent().map(folder_videos).unwrap_or_default();
        let Some(i) = list.iter().position(|x| *x == cur) else { return };
        let j = i as i32 + d;
        if j >= 0 && (j as usize) < list.len() {
            self.load(&list[j as usize]);
        }
    }

    fn toggle_fullscreen(&self) {
        if self.window.is_fullscreen() {
            self.window.unfullscreen();
        } else {
            self.window.fullscreen();
        }
    }

    /// The mini player: a small window with the video and its controls only.
    fn set_compact(&self, on: bool) {
        self.compact.set(on);
        self.toolbar.set_visible(!on);
        self.line.set_visible(!on);
        for w in [self.summary_b.upcast_ref::<gtk::Widget>(), self.captions_b.upcast_ref(), self.speed_b.upcast_ref(), self.time.upcast_ref()] {
            w.set_visible(!on);
        }
        if on {
            self.controls.add_css_class("compacta");
            self.window.set_default_size(560, 315);
        } else {
            self.controls.remove_css_class("compacta");
            let (w, h) = crate::superficies::tamanho_de_janela(1500, 910);
            self.window.set_default_size(w, h);
        }
        if debug() {
            eprintln!("debug video: mini player {on}");
        }
    }

    fn wake(&self) {
        self.last_motion.set(glib::monotonic_time());
        if self.controls.has_css_class("oculta") {
            self.controls.remove_css_class("oculta");
            self.controls.set_can_target(true);
            self.stage.set_cursor(None::<&gdk::Cursor>);
        }
    }

    fn tick(self: &Rc<Self>) {
        if let Some(pb) = self.pipeline.borrow().clone() {
            if !self.dragging.get() && !self.ended.get() {
                if let Some(p) = pb.query_position::<gst::ClockTime>() {
                    self.position.set(p.nseconds() as f64 / 1e9);
                }
            }
        }
        self.time.set_text(&format!("{} / {}", clock(self.position.get()), clock(self.length.get())));
        self.timeline.queue_draw();
        // the controls fade 2 s after the pointer stops, while the video plays (not over them, not in the request)
        let idle = (glib::monotonic_time() - self.last_motion.get()) as f64 / 1e6;
        let open_menu = [&self.volume_b, &self.speed_b].iter().any(|b| b.popover().is_some_and(|p| p.is_visible()));
        let hide = self.playing.get() && idle > 2.0 && !self.over_controls.get() && !self.dragging.get() && !open_menu;
        if hide && !self.controls.has_css_class("oculta") {
            self.controls.add_css_class("oculta");
            self.controls.set_can_target(false);
            self.stage.set_cursor_from_name(Some("none"));
            if debug() {
                eprintln!("debug video: controles somem");
            }
        }
        self.captions_tick();
    }

    /// The design's timeline: a thin line, the part played in lavender, the chapters' marks, the knob.
    fn draw_timeline(&self, cr: &cairo::Context, w: f64, h: f64) {
        let len = self.length.get();
        let frac = if len > 0.0 { (self.position.get() / len).clamp(0.0, 1.0) } else { 0.0 };
        let y = h / 2.0 - 2.0;
        let round = |x: f64, w: f64| {
            cr.new_sub_path();
            cr.arc(x + 2.0, y + 2.0, 2.0, std::f64::consts::FRAC_PI_2, 1.5 * std::f64::consts::PI);
            cr.arc(x + w - 2.0, y + 2.0, 2.0, -std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
            cr.close_path();
        };
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.2);
        round(0.0, w);
        let _ = cr.fill();
        let (r, g, b) = crate::tema::rgb("#A5A1E2");
        cr.set_source_rgb(r, g, b);
        round(0.0, (w * frac).max(4.0));
        let _ = cr.fill();
        if len > 0.0 {
            for (s, _) in self.chapters.borrow().iter().filter(|(s, _)| *s > 0.5) {
                let x = (s / len * w).round();
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.85);
                cr.rectangle(x - 1.0, h / 2.0 - 4.0, 2.0, 8.0);
                let _ = cr.fill();
            }
        }
        let x = (w * frac).clamp(7.0, w - 7.0);
        cr.set_source_rgb(0xDC as f64 / 255.0, 0xD3 as f64 / 255.0, 0xD3 as f64 / 255.0);
        cr.arc(x, h / 2.0, 7.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }

    fn say(&self, title: &str, text: &str) {
        self.card_title.set_text(title);
        self.card_text.set_text(text);
        self.card_text.set_visible(!text.is_empty());
        self.card.set_visible(true);
    }

    // ------------------------------------------------------------------------------------------- the agent

    fn read_cache(&self, path: &Path) {
        let Ok(s) = std::fs::read_to_string(media::cache_file(path)) else { return };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) else { return };
        let heard: Vec<(f64, String)> = v["fala"].as_array().into_iter().flatten()
            .map(|c| (c["t"].as_f64().unwrap_or(0.0), c["texto"].as_str().unwrap_or("").to_string())).collect();
        *self.heard.borrow_mut() = heard;
    }

    fn write_cache(&self) {
        let Some(path) = self.path.borrow().clone() else { return };
        let v = serde_json::json!({
            "capitulos": self.chapters.borrow().iter().map(|(t, n)| serde_json::json!({"t": t, "titulo": n})).collect::<Vec<_>>(),
            "fala": self.heard.borrow().iter().map(|(t, n)| serde_json::json!({"t": t, "texto": n})).collect::<Vec<_>>(),
        });
        let f = media::cache_file(&path);
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(f, v.to_string());
    }

    /// The chapters: frames taken across the video, the scene changes between them, each scene named by the local
    /// vision model from its middle frame (kept, so a video opened again has them at once).
    fn find_chapters(self: &Rc<Self>, path: &Path) {
        // (known already)
        if let Ok(v) = std::fs::read_to_string(media::cache_file(path)).map(|s| serde_json::from_str::<serde_json::Value>(&s).unwrap_or_default()) {
            let ch: Vec<(f64, String)> = v["capitulos"].as_array().into_iter().flatten()
                .map(|c| (c["t"].as_f64().unwrap_or(0.0), c["titulo"].as_str().unwrap_or("").to_string())).collect();
            if !ch.is_empty() {
                self.show_chapters(ch, true);
                return;
            }
        }
        let path = path.to_path_buf();
        let gen = self.generation.get();
        let bin = crate::voz::agent_binary();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || find_chapters_now(&path, &bin), move |r| {
            let Some(p) = w.upgrade() else { return };
            if p.generation.get() != gen {
                return;
            }
            match r {
                Ok(ch) => {
                    p.show_chapters(ch, false);
                    p.write_cache();
                }
                Err(e) => eprintln!("kalango-shell: vídeo: capítulos: {e}"),
            }
        });
    }

    fn show_chapters(self: &Rc<Self>, ch: Vec<(f64, String)>, cached: bool) {
        if debug() {
            eprintln!("debug video: capítulos ({}){}: {}", ch.len(), if cached { " guardados" } else { "" },
                ch.iter().map(|(t, n)| format!("{} «{n}»", clock(*t))).collect::<Vec<_>>().join("; "));
        }
        *self.chapters.borrow_mut() = ch;
        self.timeline.queue_draw();
        self.fill_menu();
    }

    fn set_captions(self: &Rc<Self>, on: bool) {
        self.captions_on.set(on);
        if on {
            self.captions_b.add_css_class("ativo");
        } else {
            self.captions_b.remove_css_class("ativo");
            self.caption.set_visible(false);
        }
        if let Some(i) = glyph_of(&self.captions_b) {
            set_icon(&i, "doc", if on { "#1C1D24" } else { "#E2E2E6" });
        }
        if debug() {
            eprintln!("debug video: legendas {on}");
        }
        self.captions_tick();
    }

    /// The sound of the video, read once (for the captions and the summary).
    fn with_sound(self: &Rc<Self>, then: impl FnOnce(&Rc<VideoPlayer>, Arc<Vec<i16>>) + 'static) {
        if let Some(s) = self.sound.borrow().clone() {
            return then(self, s);
        }
        // (already being read: it waits for the same reading)
        self.sound_waiters.borrow_mut().push(Box::new(then));
        if self.sound_loading.replace(true) {
            return;
        }
        let Some(path) = self.path.borrow().clone() else { return };
        let gen = self.generation.get();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || media::audio(&path), move |r| {
            let Some(p) = w.upgrade() else { return };
            if p.generation.get() != gen {
                return;
            }
            p.sound_loading.set(false);
            let s = Arc::new(r.unwrap_or_default());
            if debug() {
                eprintln!("debug video: som lido ({} s)", s.len() / media::RATE);
            }
            *p.sound.borrow_mut() = Some(s.clone());
            let waiters = std::mem::take(&mut *p.sound_waiters.borrow_mut());
            for f in waiters {
                f(&p, s.clone());
            }
        });
    }

    /// Live captions: the stretch playing and the next are transcribed ahead by the local audio model.
    fn captions_tick(self: &Rc<Self>) {
        if !self.captions_on.get() || self.path.borrow().is_none() {
            return;
        }
        let k = (self.position.get() / CAPTION as f64) as usize;
        let text = self.said.borrow().get(&k).cloned().unwrap_or_default();
        self.caption.set_text(&text);
        self.caption.set_visible(!text.is_empty());
        if self.transcribing.get() {
            return;
        }
        let Some(sound) = self.sound.borrow().clone() else {
            self.with_sound(|p, _| p.captions_tick());
            return;
        };
        let total = sound.len().div_ceil(CAPTION * media::RATE);
        let Some(j) = [k, k + 1].into_iter().find(|j| *j < total && !self.said.borrow().contains_key(j)) else { return };
        let piece: Vec<i16> = sound[j * CAPTION * media::RATE..((j + 1) * CAPTION * media::RATE).min(sound.len())].to_vec();
        if media::silent(&piece) {
            self.said.borrow_mut().insert(j, String::new());
            return;
        }
        self.transcribing.set(true);
        let gen = self.generation.get();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || transcribe(&piece), move |r: Result<String, String>| {
            let Some(p) = w.upgrade() else { return };
            p.transcribing.set(false);
            if p.generation.get() != gen {
                return;
            }
            let text = r.unwrap_or_else(|e| {
                eprintln!("kalango-shell: vídeo: legenda: {e}");
                String::new()
            });
            if debug() {
                eprintln!("debug video: legenda {}–{} «{}»", clock((j * CAPTION) as f64), clock(((j + 1) * CAPTION) as f64), text);
            }
            p.said.borrow_mut().insert(j, text);
            p.captions_tick();
        });
    }

    /// What is said in the video, in 30 s pieces (the first 15 minutes), for the summary and the requests.
    fn listen(self: &Rc<Self>, then: impl FnOnce(&Rc<VideoPlayer>) + 'static) {
        if !self.heard.borrow().is_empty() {
            return then(self);
        }
        let gen = self.generation.get();
        self.with_sound(move |p, sound| {
            let total = sound.len().div_ceil(HEARD * media::RATE).min(30);
            if total == 0 {
                return then(p);
            }
            p.say(tr("Summary"), &trf("Listening to the video… ({n} parts)", &[("n", total.to_string())]));
            let w = Rc::downgrade(p);
            crate::lateral::em_segundo_plano(move || {
                let mut out = Vec::new();
                for j in 0..total {
                    let piece = &sound[j * HEARD * media::RATE..((j + 1) * HEARD * media::RATE).min(sound.len())];
                    if media::silent(piece) {
                        continue;
                    }
                    if let Ok(t) = transcribe(piece) {
                        if !t.trim().is_empty() {
                            out.push(((j * HEARD) as f64, t));
                        }
                    }
                }
                out
            }, move |heard: Vec<(f64, String)>| {
                let Some(p) = w.upgrade() else { return };
                if p.generation.get() != gen {
                    return;
                }
                if debug() {
                    eprintln!("debug video: ouvido ({} trechos com fala)", heard.len());
                }
                *p.heard.borrow_mut() = heard;
                p.write_cache();
                then(&p);
            });
        });
    }

    fn about(&self) -> serde_json::Value {
        let name = self.path.borrow().as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        // (what was heard for the summary, or else the live captions so far)
        let mut said: Vec<(f64, String)> = self.heard.borrow().clone();
        if said.is_empty() {
            said = self.said.borrow().iter().filter(|(_, t)| !t.is_empty()).map(|(k, t)| ((*k * CAPTION) as f64, t.clone())).collect();
        }
        serde_json::json!({
            "nome": name, "duracao": self.length.get(), "agora": self.position.get(),
            "capitulos": self.chapters.borrow().iter().map(|(t, n)| serde_json::json!({"t": t, "titulo": n})).collect::<Vec<_>>(),
            "fala": said.iter().map(|(t, n)| serde_json::json!({"t": t, "texto": n})).collect::<Vec<_>>(),
        })
    }

    /// "Resumir o vídeo": what is said in it and its chapters, summed up by the local model.
    fn summarize(self: &Rc<Self>) {
        if self.path.borrow().is_none() {
            return;
        }
        self.say(tr("Summary"), tr("Reading the video…"));
        let gen = self.generation.get();
        self.listen(move |p| {
            if p.generation.get() != gen {
                return;
            }
            p.say(tr("Summary"), tr("Summing it up…"));
            let input = p.about();
            let w = Rc::downgrade(p);
            crate::lateral::em_segundo_plano(move || ask_agent("video-resumo", &input), move |r| {
                let Some(p) = w.upgrade() else { return };
                if p.generation.get() != gen {
                    return;
                }
                match r {
                    Ok(v) => {
                        let t = v["resumo"].as_str().unwrap_or("").trim().to_string();
                        if debug() {
                            eprintln!("debug video: resumo ({} caracteres)", t.chars().count());
                        }
                        p.say(tr("Summary"), &t);
                    }
                    Err(e) => p.say(tr("The summary could not be made"), &e),
                }
            });
        });
    }

    /// A request about the video: a jump, or an answer in the card.
    fn request(self: &Rc<Self>, text: String) {
        if self.path.borrow().is_none() {
            return;
        }
        self.say(&text, tr("The agent is looking…"));
        let input = {
            let mut v = self.about();
            v["pedido"] = serde_json::json!(text);
            v
        };
        let gen = self.generation.get();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || ask_agent("video-pedido", &input), move |r| {
            let Some(p) = w.upgrade() else { return };
            if p.generation.get() != gen {
                return;
            }
            match r {
                Ok(v) if v["acao"] == "pular" => {
                    let s = v["segundos"].as_f64().unwrap_or(0.0);
                    if debug() {
                        eprintln!("debug video: pedido → pular para {}s", s.round());
                    }
                    p.seek_to(s, true);
                    if !p.playing.get() {
                        p.toggle();
                    }
                    p.card.set_visible(false);
                }
                Ok(v) => {
                    let t = v["texto"].as_str().unwrap_or("").to_string();
                    if debug() {
                        eprintln!("debug video: pedido → resposta ({} caracteres)", t.chars().count());
                    }
                    p.card_text.set_text(&t);
                }
                Err(e) => p.say(tr("The agent could not answer"), &e),
            }
        });
    }
}

/// The local audio model's words for a stretch of sound.
fn transcribe(samples: &[i16]) -> Result<String, String> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let f = std::env::temp_dir().join(format!("kalango-video-fala-{}-{}.wav", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&f, media::wav(samples)).map_err(|e| e.to_string())?;
    let o = std::process::Command::new(crate::voz::agent_binary()).args(["--json", "transcrever"]).arg(&f).stderr(std::process::Stdio::piped()).output();
    let _ = std::fs::remove_file(&f);
    let o = o.map_err(|e| e.to_string())?;
    // (the words only: an error is said as one, never shown as a caption)
    let v = serde_json::from_slice::<serde_json::Value>(&o.stdout).unwrap_or_default();
    match v["tipo"].as_str() {
        Some("transcricao") => Ok(v["texto"].as_str().unwrap_or("").trim().to_string()),
        Some(_) => Err(v["texto"].as_str().unwrap_or("").to_string()),
        None => Err(String::from_utf8_lossy(&o.stderr).trim().lines().last().unwrap_or("").to_string()),
    }
}

fn ask_agent(cmd: &str, input: &serde_json::Value) -> Result<serde_json::Value, String> {
    use std::io::Write;
    let mut c = std::process::Command::new(crate::voz::agent_binary()).arg(cmd).stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    c.stdin.take().ok_or("stdin")?.write_all(input.to_string().as_bytes()).map_err(|e| e.to_string())?;
    let o = c.wait_with_output().map_err(|e| e.to_string())?;
    serde_json::from_slice::<serde_json::Value>(&o.stdout).map_err(|_| String::from_utf8_lossy(&o.stderr).trim().lines().last().unwrap_or("").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_and_kinds() {
        assert_eq!(clock(252.4), "04:12");
        assert_eq!(clock(663.0), "11:03");
        assert_eq!(clock(3725.0), "1:02:05");
        assert!(is_video(Path::new("/x/viagem-praia.MP4")));
        assert!(!is_video(Path::new("/x/nota.txt")));
    }
}
