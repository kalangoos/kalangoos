//! KalangoOS's own image viewer (design "Visualizador de imagens", 58465): the picture fills the window; the tools
//! float over it and fade after 2 s without the pointer moving (180 ms); the folder's pictures in a strip below;
//! going to the next one slides. The agent's actions ("Com um pedido") work on this computer: the background out,
//! sharper, colour and tone variations (image_ops.rs), the description and the text in the picture (the local vision
//! model) — what changes the picture shows before and after with a curtain that can be dragged, and is kept as a copy.

use crate::conversa::ui_icon;
use crate::image_ops::{self, Rgba};
use crate::superficies::{caixa, rotulo};
use crate::Shell;
use gtk::gdk::prelude::GdkCairoContextExt;
use gtk::gdk_pixbuf::{Colorspace, Pixbuf, PixbufRotation};
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

const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff", "svg", "ico", "avif", "heic", "jxl"];
/// The thumbnails kept around the one in view.
const STRIP_SPAN: usize = 30;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

pub fn is_image(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

/// The pictures in a folder, by name.
fn folder_images(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| p.is_file() && is_image(p) && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect();
    v.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default());
    v
}

fn to_rgba(pb: &Pixbuf) -> Rgba {
    let pb = if pb.has_alpha() { pb.clone() } else { pb.add_alpha(false, 0, 0, 0).unwrap_or_else(|_| pb.clone()) };
    let (w, h, rs) = (pb.width() as usize, pb.height() as usize, pb.rowstride() as usize);
    let bytes = pb.read_pixel_bytes();
    let mut data = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let start = y * rs;
        data.extend_from_slice(&bytes[start..(start + w * 4).min(bytes.len())]);
    }
    data.resize(w * h * 4, 0);
    Rgba { width: w, height: h, data }
}

fn from_rgba(r: &Rgba) -> Pixbuf {
    Pixbuf::from_bytes(&glib::Bytes::from(&r.data[..]), Colorspace::Rgb, true, 8, r.width as i32, r.height as i32, (r.width * 4) as i32)
}

/// A picture ready to draw (cairo's own surface: drawn each frame without converting again).
fn surface_of(r: &Rgba) -> Option<cairo::ImageSurface> {
    let pb = from_rgba(r);
    let s = cairo::ImageSurface::create(cairo::Format::ARgb32, pb.width(), pb.height()).ok()?;
    let cr = cairo::Context::new(&s).ok()?;
    cr.set_source_pixbuf(&pb, 0.0, 0.0);
    cr.paint().ok()?;
    drop(cr);
    Some(s)
}

/// Reads a picture (its orientation from the camera applied).
fn load(p: &Path) -> Result<Rgba, String> {
    let pb = Pixbuf::from_file(p).map_err(|e| e.to_string())?;
    let pb = pb.apply_embedded_orientation().unwrap_or(pb);
    Ok(to_rgba(&pb))
}

/// What the file says about where the picture came from: made by an AI (the generators' own marks in its metadata),
/// a screenshot, a camera's photo.
pub fn origin_of(p: &Path) -> &'static str {
    let mut head = vec![0u8; 256 * 1024];
    let n = std::fs::File::open(p).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
    let text = String::from_utf8_lossy(&head[..n]).to_lowercase();
    let ai = ["trainedalgorithmicmedia", "stable diffusion", "comfyui", "midjourney", "dall-e", "dall·e", "firefly", "higgsfield", "c2pa",
        "negative prompt", "\"prompt\"", "parameters\0", "gpt-image", "imagen", "flux"];
    let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ai.iter().any(|k| text.contains(k)) {
        n_("made by AI")
    } else if name.starts_with("captura") || name.starts_with("screenshot") {
        n_("screenshot")
    } else if text.contains("exif") && ["canon", "nikon", "sony", "apple", "samsung", "xiaomi", "motorola", "fujifilm", "google"].iter().any(|k| text.contains(k)) {
        n_("camera photo")
    } else {
        ""
    }
}

fn n_(s: &'static str) -> &'static str {
    kalango_i18n::n_(s)
}

/// A copy's name next to the original: "nave (sem fundo).png", "nave (sem fundo) 2.png"…
fn copy_path(original: &Path, what: &str) -> PathBuf {
    let dir = original.parent().unwrap_or(Path::new("."));
    let stem = original.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "imagem".into());
    let mut p = dir.join(format!("{stem} ({what}).png"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{stem} ({what}) {n}.png"));
        n += 1;
    }
    p
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    View,
    Crop,
}

/// An edit waiting to be kept: what it is (its name in the copy's file name) and its pixels.
struct Pending {
    what: String,
    after: Rgba,
    surface: Option<cairo::ImageSurface>,
}

pub struct ImageViewer {
    window: gtk::ApplicationWindow,
    title: gtk::Label,
    canvas: gtk::DrawingArea,
    floating: Vec<gtk::Widget>,
    side: gtk::Box,
    details: [gtk::Label; 4],
    result: gtk::Box,
    strip: gtk::Box,
    strip_scroll: gtk::ScrolledWindow,
    zoom_label: gtk::Label,
    edit_bar: gtk::Box,
    edit_label: gtk::Label,
    crop_bar: gtk::Box,
    ask: gtk::Entry,
    empty: gtk::Label,
    files: RefCell<Vec<PathBuf>>,
    current: Cell<usize>,
    image: RefCell<Option<Rgba>>,
    surface: RefCell<Option<cairo::ImageSurface>>,
    pending: RefCell<Option<Pending>>,
    curtain: Cell<f64>,
    /// None: fit to the window; Some(z): that zoom
    zoom: Cell<Option<f64>>,
    pan: Cell<(f64, f64)>,
    slide: Cell<f64>,
    mode: Cell<Mode>,
    crop: Cell<Option<(f64, f64, f64, f64)>>,
    drag_start: Cell<(f64, f64, f64, f64)>,
    adjust: Cell<(f64, f64, f64)>,
    hide_timer: RefCell<Option<glib::SourceId>>,
    over_tools: Cell<bool>,
    busy: Cell<bool>,
    /// where the pointer is on the canvas (the wheel zooms there)
    pointer: Cell<(f64, f64)>,
    shell: Weak<Shell>,
}

thread_local! {
    static VIEWER: RefCell<Option<Rc<ImageViewer>>> = const { RefCell::new(None) };
}

/// Opens the viewer on a picture (or the system's chooser when none is given).
pub fn open(shell: &Rc<Shell>, path: Option<PathBuf>) {
    let v = VIEWER.with(|v| v.borrow().clone()).unwrap_or_else(|| {
        let app = gio::Application::default().and_downcast::<gtk::Application>().expect("app");
        let v = ImageViewer::new(&app, shell);
        VIEWER.with(|x| *x.borrow_mut() = Some(v.clone()));
        v
    });
    if !v.window.is_visible() {
        let (w, h) = crate::superficies::tamanho_de_janela(1500, 910);
        v.window.set_default_size(w, h);
    }
    v.window.present();
    match path {
        Some(p) => v.show_file(&p),
        None if v.files.borrow().is_empty() => v.choose(),
        None => {}
    }
}

fn tool_button(glyph: &str, tip: &str, danger: bool) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("img-ferramenta");
    b.set_child(Some(&ui_icon(glyph, if danger { "#EFA2A6" } else { "#C9CAD1" }, 16)));
    b.set_tooltip_text(Some(tip));
    b
}

impl ImageViewer {
    fn new(app: &gtk::Application, shell: &Rc<Shell>) -> Rc<ImageViewer> {
        let (w, h) = crate::superficies::tamanho_de_janela(1500, 910);
        let window = gtk::ApplicationWindow::builder().application(app).title(tr("Images")).default_width(w).default_height(h).build();
        // (one window for the app, kept: closing hides it and opening shows it again — a closed GTK window shown again
        // could not be closed any more, and it kept text input from the other windows)
        window.set_hide_on_close(true);
        window.add_css_class("ed-janela");
        let sem_barra = gtk::Box::new(Orientation::Horizontal, 0);
        sem_barra.set_visible(false);
        window.set_titlebar(Some(&sem_barra));
        // the bar: the app, the picture's name, the agent's field, the menu, the window's buttons
        let bar = caixa(Orientation::Horizontal, 10, "ed-barra");
        bar.append(&crate::apps::imagem_kalango("imagens", 24));
        let title = rotulo(tr("Images"), "img-titulo");
        title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        title.set_max_width_chars(1);
        title.set_hexpand(true);
        title.set_xalign(0.0);
        bar.append(&title);
        let ask = gtk::Entry::new();
        ask.add_css_class("ed-pedir");
        ask.set_size_request(300, 32);
        ask.set_valign(Align::Center);
        ask.set_placeholder_text(Some(tr("Ask: “remove the background”")));
        ask.set_primary_icon_paintable(ui_icon("spark", "#A5A1E2", 13).paintable().as_ref());
        let ask_box = gtk::Overlay::new();
        ask_box.set_child(Some(&ask));
        let key = rotulo("Ctrl K", "tb-atalho");
        key.set_halign(Align::End);
        key.set_valign(Align::Center);
        key.set_margin_end(8);
        key.set_can_target(false);
        ask_box.add_overlay(&key);
        bar.append(&ask_box);
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

        // the picture
        let canvas = gtk::DrawingArea::new();
        canvas.set_hexpand(true);
        canvas.set_vexpand(true);
        canvas.set_focusable(true);
        canvas.add_css_class("img-tela");
        let over = gtk::Overlay::new();
        over.set_child(Some(&canvas));
        let empty = rotulo(tr("No picture open"), "img-vazio");
        empty.set_can_target(false);
        over.add_overlay(&empty);

        // the tools, on the left
        let tools = caixa(Orientation::Vertical, 4, "img-ferramentas");
        tools.add_css_class("img-flutuante");
        tools.set_halign(Align::Start);
        tools.set_valign(Align::Center);
        tools.set_margin_start(20);
        let t_zoom = tool_button("busca", tr("Zoom: fit or 100 %"), false);
        let t_crop = tool_button("recortar", tr("Crop"), false);
        let t_rotate = tool_button("reiniciar", tr("Rotate to the right"), false);
        let t_adjust = gtk::MenuButton::new();
        t_adjust.add_css_class("img-ferramenta-menu");
        t_adjust.set_child(Some(&ui_icon("ajustes", "#C9CAD1", 16)));
        t_adjust.set_tooltip_text(Some(tr("Brightness, contrast and colour")));
        let t_share = gtk::MenuButton::new();
        t_share.add_css_class("img-ferramenta-menu");
        t_share.set_child(Some(&ui_icon("compartilhar", "#C9CAD1", 16)));
        t_share.set_tooltip_text(Some(tr("Share")));
        let t_delete = tool_button("lixeira", tr("Move to the trash"), true);
        for (b, n) in [(t_zoom.upcast_ref::<gtk::Widget>(), "zoom"), (t_crop.upcast_ref(), "recortar"), (t_rotate.upcast_ref(), "girar"),
            (t_adjust.upcast_ref(), "ajustes"), (t_share.upcast_ref(), "compartilhar"), (t_delete.upcast_ref(), "excluir")] {
            crate::central::posicao_debug_sempre(&format!("imagens ferramenta {n}"), b);
            tools.append(b);
        }
        over.add_overlay(&tools);

        // the agent's actions and the details, on the right
        let side = caixa(Orientation::Vertical, 0, "img-lado");
        side.add_css_class("img-flutuante");
        side.set_halign(Align::End);
        side.set_valign(Align::Start);
        side.set_margin_end(20);
        side.set_margin_top(90);
        side.set_size_request(250, -1);
        side.set_hexpand(false);
        let acts = caixa(Orientation::Vertical, 2, "img-lado-bloco");
        let head = caixa(Orientation::Horizontal, 6, "");
        head.append(&ui_icon("spark", "#A5A1E2", 12));
        head.append(&rotulo(tr("WITH A REQUEST"), "img-lado-titulo"));
        acts.append(&head);
        let mut actions: Vec<(&'static str, gtk::Button)> = Vec::new();
        for (id, glyph, name) in [("fundo", "recortar", tr("Remove the background")), ("descrever", "olho", tr("Describe the picture")),
            ("nitidez", "sol", tr("Sharpen")), ("texto", "doc", tr("Copy the text in the picture")), ("variacoes", "imagem", tr("Create variations"))] {
            let b = gtk::Button::new();
            b.add_css_class("img-acao");
            let r = caixa(Orientation::Horizontal, 10, "");
            r.append(&ui_icon(glyph, "#8E8F99", 15));
            let l = rotulo(name, "");
            l.set_xalign(0.0);
            r.append(&l);
            b.set_child(Some(&r));
            crate::central::posicao_debug_sempre(&format!("imagens acao {id}"), b.upcast_ref());
            acts.append(&b);
            actions.push((id, b));
        }
        side.append(&acts);
        // what an action answered (a description, the text read, the variations)
        let result = caixa(Orientation::Vertical, 8, "img-lado-bloco");
        result.set_visible(false);
        side.append(&result);
        let det = caixa(Orientation::Vertical, 6, "img-lado-bloco");
        det.append(&rotulo_left(tr("DETAILS"), "img-lado-titulo"));
        let mut labels = Vec::new();
        for name in [tr("Size"), tr("Weight"), tr("Made on"), tr("Origin")] {
            let r = caixa(Orientation::Horizontal, 8, "");
            let n = rotulo_left(name, "img-detalhe-nome");
            n.set_hexpand(true);
            r.append(&n);
            let v = rotulo("—", "img-detalhe-valor");
            v.set_xalign(1.0);
            r.append(&v);
            det.append(&r);
            labels.push(v);
        }
        side.append(&det);
        over.add_overlay(&side);

        // the folder's pictures, below
        let strip = caixa(Orientation::Horizontal, 8, "");
        let strip_scroll = gtk::ScrolledWindow::new();
        strip_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        strip_scroll.set_propagate_natural_width(true);
        strip_scroll.set_child(Some(&strip));
        strip_scroll.add_css_class("img-tira");
        strip_scroll.add_css_class("img-flutuante");
        strip_scroll.set_halign(Align::Center);
        strip_scroll.set_valign(Align::End);
        strip_scroll.set_margin_bottom(14);
        over.add_overlay(&strip_scroll);

        // the zoom, bottom left
        let zoom_box = caixa(Orientation::Horizontal, 2, "img-zoom");
        zoom_box.add_css_class("img-flutuante");
        zoom_box.set_halign(Align::Start);
        zoom_box.set_valign(Align::End);
        zoom_box.set_margin_start(20);
        zoom_box.set_margin_bottom(30);
        let z_out = tool_button("minimizar", tr("Zoom out"), false);
        let zoom_label = rotulo("100%", "img-zoom-valor");
        let z_in = tool_button("mais", tr("Zoom in"), false);
        zoom_box.append(&z_out);
        zoom_box.append(&zoom_label);
        zoom_box.append(&z_in);
        over.add_overlay(&zoom_box);

        // an edit waiting: keep it as a copy, or let it go
        let edit_bar = caixa(Orientation::Horizontal, 10, "img-edicao");
        edit_bar.set_halign(Align::Center);
        edit_bar.set_valign(Align::End);
        edit_bar.set_margin_bottom(116);
        let edit_label = rotulo("", "img-edicao-texto");
        edit_bar.append(&edit_label);
        let keep = gtk::Button::with_label(tr("Save a copy"));
        keep.add_css_class("cfg-botao-principal");
        keep.add_css_class("pequeno");
        let drop_b = gtk::Button::with_label(tr("Discard"));
        drop_b.add_css_class("cfg-botao-texto");
        edit_bar.append(&keep);
        edit_bar.append(&drop_b);
        crate::central::posicao_debug_sempre("imagens salvar", keep.upcast_ref());
        crate::central::posicao_debug_sempre("imagens descartar", drop_b.upcast_ref());
        edit_bar.set_visible(false);
        over.add_overlay(&edit_bar);
        // cropping: drag over the picture, then crop
        let crop_bar = caixa(Orientation::Horizontal, 10, "img-edicao");
        crop_bar.set_halign(Align::Center);
        crop_bar.set_valign(Align::End);
        crop_bar.set_margin_bottom(116);
        crop_bar.append(&rotulo(tr("Drag over the picture to choose what stays"), "img-edicao-texto"));
        let do_crop = gtk::Button::with_label(tr("Crop"));
        do_crop.add_css_class("cfg-botao-principal");
        do_crop.add_css_class("pequeno");
        let no_crop = gtk::Button::with_label(tr("Cancel"));
        no_crop.add_css_class("cfg-botao-texto");
        crop_bar.append(&do_crop);
        crop_bar.append(&no_crop);
        crate::central::posicao_debug_sempre("imagens recortar confirmar", do_crop.upcast_ref());
        crop_bar.set_visible(false);
        over.add_overlay(&crop_bar);

        let root = caixa(Orientation::Vertical, 0, "");
        root.append(&handle);
        root.append(&over);
        window.set_child(Some(&root));

        let details: [gtk::Label; 4] = labels.try_into().unwrap_or_else(|_| unreachable!());
        let floating: Vec<gtk::Widget> = vec![tools.upcast(), side.clone().upcast(), strip_scroll.clone().upcast(), zoom_box.upcast()];
        let v = Rc::new(ImageViewer {
            window, title, canvas, floating, side, details, result, strip, strip_scroll, zoom_label, edit_bar, edit_label, crop_bar, ask, empty,
            files: RefCell::default(), current: Cell::new(0), image: RefCell::default(), surface: RefCell::default(), pending: RefCell::default(),
            curtain: Cell::new(0.5), zoom: Cell::new(None), pan: Cell::new((0.0, 0.0)), slide: Cell::new(0.0), mode: Cell::new(Mode::View),
            crop: Cell::new(None), drag_start: Cell::new((0.0, 0.0, 0.0, 0.0)), adjust: Cell::new((0.0, 0.0, 1.0)), hide_timer: RefCell::default(),
            over_tools: Cell::new(false), busy: Cell::new(false), pointer: Cell::new((0.0, 0.0)), shell: Rc::downgrade(shell),
        });
        v.wire(&over, &items, &pop, &actions, &t_zoom, &t_crop, &t_rotate, &t_adjust, &t_share, &t_delete, &z_out, &z_in, &keep, &drop_b, &do_crop, &no_crop);
        v
    }

    #[allow(clippy::too_many_arguments)]
    fn wire(self: &Rc<Self>, over: &gtk::Overlay, items: &gtk::Box, pop: &gtk::Popover, actions: &[(&'static str, gtk::Button)], t_zoom: &gtk::Button,
        t_crop: &gtk::Button, t_rotate: &gtk::Button, t_adjust: &gtk::MenuButton, t_share: &gtk::MenuButton, t_delete: &gtk::Button, z_out: &gtk::Button,
        z_in: &gtk::Button, keep: &gtk::Button, drop_b: &gtk::Button, do_crop: &gtk::Button, no_crop: &gtk::Button) {
        let w = Rc::downgrade(self);
        self.canvas.set_draw_func(move |_, cr, width, height| {
            if let Some(v) = w.upgrade() {
                v.draw(cr, f64::from(width), f64::from(height));
            }
        });
        // the tools show while the pointer moves, and fade 2 s after it stops (not while it is over them)
        let motion = gtk::EventControllerMotion::new();
        let w = Rc::downgrade(self);
        motion.connect_motion(move |_, _, _| {
            if let Some(v) = w.upgrade() {
                v.show_tools();
            }
        });
        over.add_controller(motion);
        let at = gtk::EventControllerMotion::new();
        let w = Rc::downgrade(self);
        at.connect_motion(move |_, x, y| {
            if let Some(v) = w.upgrade() {
                v.pointer.set((x, y));
            }
        });
        self.canvas.add_controller(at);
        for f in &self.floating {
            let m = gtk::EventControllerMotion::new();
            let w = Rc::downgrade(self);
            m.connect_enter(move |_, _, _| {
                if let Some(v) = w.upgrade() {
                    v.over_tools.set(true);
                }
            });
            let w = Rc::downgrade(self);
            m.connect_leave(move |_| {
                if let Some(v) = w.upgrade() {
                    v.over_tools.set(false);
                }
            });
            f.add_controller(m);
        }
        // the wheel zooms where the pointer is; sideways (a touchpad), the next or the previous picture
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let w = Rc::downgrade(self);
        let swipe = Rc::new(Cell::new(0.0f64));
        scroll.connect_scroll(move |_, dx, dy| {
            let Some(v) = w.upgrade() else { return glib::Propagation::Proceed };
            if dx.abs() > dy.abs() {
                swipe.set(swipe.get() + dx);
                if swipe.get().abs() > 3.0 {
                    v.step(if swipe.get() > 0.0 { 1 } else { -1 });
                    swipe.set(0.0);
                }
                return glib::Propagation::Stop;
            }
            v.zoom_by(if dy < 0.0 { 1.15 } else { 1.0 / 1.15 }, Some(v.pointer.get()));
            glib::Propagation::Stop
        });
        self.canvas.add_controller(scroll);
        // dragging: pans the zoomed picture, moves the curtain, or chooses the crop
        let drag = gtk::GestureDrag::new();
        let w = Rc::downgrade(self);
        drag.connect_drag_begin(move |_, x, y| {
            if let Some(v) = w.upgrade() {
                let (px, py) = v.pan.get();
                v.drag_start.set((x, y, px, py));
                if v.mode.get() == Mode::Crop {
                    if let Some((ix, iy)) = v.to_image(x, y) {
                        v.crop.set(Some((ix, iy, ix, iy)));
                    }
                }
            }
        });
        let w = Rc::downgrade(self);
        drag.connect_drag_update(move |_, dx, dy| {
            let Some(v) = w.upgrade() else { return };
            let (x0, y0, px, py) = v.drag_start.get();
            if v.mode.get() == Mode::Crop {
                if let (Some((ax, ay, _, _)), Some((bx, by))) = (v.crop.get(), v.to_image(x0 + dx, y0 + dy)) {
                    v.crop.set(Some((ax, ay, bx, by)));
                }
            } else if v.pending.borrow().is_some() {
                let wd = f64::from(v.canvas.width()).max(1.0);
                v.curtain.set(((x0 + dx) / wd).clamp(0.0, 1.0));
            } else if v.zoom.get().is_some() {
                v.pan.set((px + dx, py + dy));
            }
            v.canvas.queue_draw();
        });
        self.canvas.add_controller(drag);
        // a double click: fit, or 100 %
        let click = gtk::GestureClick::new();
        let w = Rc::downgrade(self);
        click.connect_pressed(move |_, n, x, y| {
            if let Some(v) = w.upgrade() {
                v.canvas.grab_focus();
                if n == 2 && v.mode.get() == Mode::View {
                    v.toggle_zoom(Some((x, y)));
                }
            }
        });
        self.canvas.add_controller(click);
        // keys: ← → pictures, + − 0 1 zoom, R rotate, Delete trash, Esc out of an edit, Ctrl+K the agent's field
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, k, _, m| {
            let Some(v) = w.upgrade() else { return glib::Propagation::Proceed };
            // (typing in the agent's field is typing: its focus is the text inside it)
            let typing = gtk::prelude::GtkWindowExt::focus(&v.window).is_some_and(|f| f.is_ancestor(&v.ask) || f == *v.ask.upcast_ref::<gtk::Widget>());
            if typing && k != gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            let ctrl = m.contains(gdk::ModifierType::CONTROL_MASK);
            match k {
                gdk::Key::k if ctrl => {
                    v.ask.grab_focus();
                }
                gdk::Key::Right | gdk::Key::Page_Down => v.step(1),
                gdk::Key::Left | gdk::Key::Page_Up => v.step(-1),
                gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => v.zoom_by(1.25, None),
                gdk::Key::minus | gdk::Key::KP_Subtract => v.zoom_by(0.8, None),
                gdk::Key::_0 => v.set_zoom(None),
                gdk::Key::_1 => v.set_zoom(Some(1.0)),
                gdk::Key::r | gdk::Key::R if !ctrl => v.rotate(),
                gdk::Key::Delete => v.trash(),
                gdk::Key::Escape => {
                    if v.mode.get() == Mode::Crop {
                        v.end_crop();
                    } else if v.pending.borrow().is_some() {
                        v.discard();
                    } else {
                        return glib::Propagation::Proceed;
                    }
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.window.add_controller(keys);

        let w = Rc::downgrade(self);
        t_zoom.connect_clicked(move |_| {
            if let Some(v) = w.upgrade() {
                v.toggle_zoom(None);
            }
        });
        let w = Rc::downgrade(self);
        z_in.connect_clicked(move |_| {
            if let Some(v) = w.upgrade() {
                v.zoom_by(1.25, None);
            }
        });
        let w = Rc::downgrade(self);
        z_out.connect_clicked(move |_| {
            if let Some(v) = w.upgrade() {
                v.zoom_by(0.8, None);
            }
        });
        let w = Rc::downgrade(self);
        t_rotate.connect_clicked(move |_| {
            if let Some(v) = w.upgrade() {
                v.rotate();
            }
        });
        let w = Rc::downgrade(self);
        t_delete.connect_clicked(move |_| {
            if let Some(v) = w.upgrade() {
                v.trash();
            }
        });
        let w = Rc::downgrade(self);
        t_crop.connect_clicked(move |_| {
            if let Some(v) = w.upgrade() {
                v.start_crop();
            }
        });
        let w = Rc::downgrade(self);
        do_crop.connect_clicked(move |_| {
            if let Some(v) = w.upgrade() {
                v.apply_crop();
            }
        });
        let w = Rc::downgrade(self);
        no_crop.connect_clicked(move |_| {
            if let Some(v) = w.upgrade() {
                v.end_crop();
            }
        });
        let w = Rc::downgrade(self);
        keep.connect_clicked(move |_| {
            if let Some(v) = w.upgrade() {
                v.keep();
            }
        });
        let w = Rc::downgrade(self);
        drop_b.connect_clicked(move |_| {
            if let Some(v) = w.upgrade() {
                v.discard();
            }
        });
        self.adjust_popover(t_adjust);
        self.share_popover(t_share);
        self.menu(items, pop);
        for (id, b) in actions {
            let (id, w) = (*id, Rc::downgrade(self));
            b.connect_clicked(move |_| {
                if let Some(v) = w.upgrade() {
                    v.action(id);
                }
            });
        }
        let w = Rc::downgrade(self);
        self.ask.connect_activate(move |e| {
            let t = e.text().trim().to_string();
            let Some(v) = w.upgrade() else { return };
            let Some(p) = v.path() else { return };
            if t.is_empty() {
                return;
            }
            e.set_text("");
            if let Some(s) = v.shell.upgrade() {
                s.right_bar().ask_agent(&trf("About the picture {path}: {request}", &[("path", p.display().to_string()), ("request", t)]));
            }
        });
        // (the window measured: the side panel waits on a narrow one, the strip as wide as there is room)
        let meter = gtk::DrawingArea::new();
        meter.set_can_target(false);
        over.add_overlay(&meter);
        over.set_measure_overlay(&meter, false);
        let w = Rc::downgrade(self);
        meter.connect_resize(move |_, width, _| {
            if let Some(v) = w.upgrade() {
                v.side.set_visible(width >= 900);
                v.strip_scroll.set_max_content_width((width - 120).max(200));
            }
        });
        // its own app id (the dock, niri's window rules, the session test know it by it)
        let app_id = |j: &gtk::ApplicationWindow| {
            if let Some(s) = j.surface().filter(|s| s.type_().name().contains("Wayland")) {
                let id = std::ffi::CString::new("os.kalango.Imagens").unwrap();
                unsafe { gdk_wayland_toplevel_set_application_id(s.as_ptr() as *mut gdk::ffi::GdkToplevel, id.as_ptr()) };
            }
        };
        self.window.connect_realize(move |j| app_id(j));
        self.window.connect_map(move |j| {
            app_id(j);
            // (the keys are the app's: the agent's field is focused only when asked, Ctrl+K)
            let j = j.clone();
            glib::idle_add_local_once(move || gtk::prelude::GtkWindowExt::set_focus(&j, None::<&gtk::Widget>));
        });
        self.show_tools();
    }

    fn path(&self) -> Option<PathBuf> {
        self.files.borrow().get(self.current.get()).cloned()
    }

    // ------------------------------------------------------------------------------------------- the tools' fade

    fn show_tools(self: &Rc<Self>) {
        for f in &self.floating {
            f.remove_css_class("oculta");
            f.set_can_target(true);
        }
        if let Some(id) = self.hide_timer.borrow_mut().take() {
            id.remove();
        }
        let w = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_secs(2), move || {
            let Some(v) = w.upgrade() else { return };
            v.hide_timer.borrow_mut().take();
            // (not while the pointer is on them, an edit waits or a menu is open)
            if v.over_tools.get() || v.pending.borrow().is_some() || v.mode.get() == Mode::Crop || v.busy.get() {
                v.show_tools();
                return;
            }
            for f in &v.floating {
                f.add_css_class("oculta");
                f.set_can_target(false);
            }
        });
        *self.hide_timer.borrow_mut() = Some(id);
    }

    // ------------------------------------------------------------------------------------------- opening

    fn choose(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        let filtros = vec![crate::arquivos::seletor::Filtro::de(tr("Pictures"), &["*.png", "*.jpg", "*.jpeg", "*.webp", "*.gif", "*.bmp", "*.svg", "*.tif", "*.tiff"])];
        crate::arquivos::seletor::abrir_arquivo(tr("Images"), tr("Open a picture"), None, filtros, None, move |p| {
            if let Some(v) = w.upgrade() {
                v.show_file(&p);
            }
        });
    }

    /// Shows a file, with its folder's pictures in the strip.
    pub fn show_file(self: &Rc<Self>, p: &Path) {
        let dir = p.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut files = folder_images(&dir);
        if !files.iter().any(|f| f == p) {
            files.push(p.to_path_buf());
        }
        let i = files.iter().position(|f| f == p).unwrap_or(0);
        *self.files.borrow_mut() = files;
        self.current.set(i);
        self.fill_strip();
        self.load_current(0.0);
    }

    fn step(self: &Rc<Self>, by: i64) {
        let n = self.files.borrow().len() as i64;
        if n < 2 || self.mode.get() == Mode::Crop {
            return;
        }
        let i = (self.current.get() as i64 + by).rem_euclid(n) as usize;
        self.current.set(i);
        self.mark_strip();
        self.load_current(by.signum() as f64);
    }

    /// Reads the picture in view (in the background) and slides it in from the side it comes from.
    fn load_current(self: &Rc<Self>, from: f64) {
        let Some(p) = self.path() else {
            self.show_empty();
            return;
        };
        self.discard_quietly();
        self.title.set_text(&trf("{name} — Images", &[("name", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())]));
        self.window.set_title(Some(&self.title.text()));
        self.empty.set_visible(false);
        self.result.set_visible(false);
        let w = Rc::downgrade(self);
        let p2 = p.clone();
        crate::lateral::em_segundo_plano(move || {
            let img = load(&p2);
            let size = std::fs::metadata(&p2).map(|m| m.len()).unwrap_or(0);
            let created = std::fs::metadata(&p2).ok().and_then(|m| m.created().or_else(|_| m.modified()).ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
            (img, size, created, origin_of(&p2))
        }, move |(img, size, created, origin): (Result<Rgba, String>, u64, Option<i64>, &'static str)| {
            let Some(v) = w.upgrade() else { return };
            if v.path().as_deref() != Some(p.as_path()) {
                return;
            }
            match img {
                Ok(img) => {
                    v.details[0].set_text(&format!("{} × {}", img.width, img.height));
                    *v.surface.borrow_mut() = surface_of(&img);
                    if debug() {
                        eprintln!("debug imagens: aberta «{}» ({}×{}, {} de {} na pasta)", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                            img.width, img.height, v.current.get() + 1, v.files.borrow().len());
                    }
                    *v.image.borrow_mut() = Some(img);
                }
                Err(e) => {
                    *v.image.borrow_mut() = None;
                    *v.surface.borrow_mut() = None;
                    v.empty.set_text(&trf("This picture could not be read: {error}", &[("error", e)]));
                    v.empty.set_visible(true);
                    v.details[0].set_text("—");
                }
            }
            v.details[1].set_text(&glib::format_size(size));
            v.details[2].set_text(&created.and_then(|s| glib::DateTime::from_unix_local(s).ok()).and_then(|d| d.format("%-d %b %Y").ok()).map(|s| s.to_string()).unwrap_or_else(|| "—".into()));
            v.details[3].set_text(if origin.is_empty() { "—" } else { tr(origin) });
            if origin == n_("made by AI") {
                v.details[3].add_css_class("accent");
            } else {
                v.details[3].remove_css_class("accent");
            }
            v.zoom.set(None);
            v.pan.set((0.0, 0.0));
            v.slide_in(from);
            v.update_zoom_label();
            v.canvas.queue_draw();
        });
    }

    fn show_empty(self: &Rc<Self>) {
        *self.image.borrow_mut() = None;
        *self.surface.borrow_mut() = None;
        self.title.set_text(tr("Images"));
        self.empty.set_text(tr("No picture open"));
        self.empty.set_visible(true);
        for d in &self.details {
            d.set_text("—");
        }
        self.canvas.queue_draw();
    }

    /// The new picture comes from the side it is on (220 ms, easing out).
    fn slide_in(self: &Rc<Self>, from: f64) {
        if from == 0.0 {
            self.slide.set(0.0);
            return;
        }
        self.slide.set(from);
        let start = glib::monotonic_time();
        let w = Rc::downgrade(self);
        self.canvas.add_tick_callback(move |_, _| {
            let Some(v) = w.upgrade() else { return glib::ControlFlow::Break };
            let t = ((glib::monotonic_time() - start) as f64 / 220_000.0).min(1.0);
            let ease = 1.0 - (1.0 - t).powi(3);
            v.slide.set(from * (1.0 - ease));
            v.canvas.queue_draw();
            if t >= 1.0 { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
        });
    }

    // ------------------------------------------------------------------------------------------- the strip

    fn fill_strip(self: &Rc<Self>) {
        while let Some(c) = self.strip.first_child() {
            self.strip.remove(&c);
        }
        let files = self.files.borrow().clone();
        let cur = self.current.get();
        let lo = cur.saturating_sub(STRIP_SPAN / 2);
        let hi = (lo + STRIP_SPAN).min(files.len());
        let mut jobs = Vec::new();
        for (i, f) in files.iter().enumerate().take(hi).skip(lo) {
            let b = gtk::Button::new();
            b.add_css_class("img-miniatura");
            let pic = gtk::Image::new();
            pic.set_pixel_size(64);
            b.set_child(Some(&pic));
            b.set_tooltip_text(f.file_name().map(|n| n.to_string_lossy().into_owned()).as_deref());
            unsafe { b.set_data("indice", i) };
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(v) = w.upgrade() {
                    let from = (i as f64 - v.current.get() as f64).signum();
                    v.current.set(i);
                    v.mark_strip();
                    v.load_current(from);
                }
            });
            crate::central::posicao_debug_sempre(&format!("imagens miniatura {}", i + 1), b.upcast_ref());
            self.strip.append(&b);
            jobs.push((f.clone(), pic));
        }
        self.mark_strip();
        // the thumbnails, read in the background one after the other
        let paths: Vec<PathBuf> = jobs.iter().map(|(p, _)| p.clone()).collect();
        let pics: Vec<gtk::Image> = jobs.into_iter().map(|(_, pic)| pic).collect();
        crate::lateral::em_segundo_plano(move || {
            // (a square from the middle of each picture, as the design's thumbnails)
            paths.iter().map(|p| {
                let pb = Pixbuf::from_file_at_scale(p, 256, 256, true).ok()?;
                let pb = pb.apply_embedded_orientation().unwrap_or(pb);
                let side = pb.width().min(pb.height());
                let sq = pb.new_subpixbuf((pb.width() - side) / 2, (pb.height() - side) / 2, side, side);
                let sq = sq.scale_simple(128, 128, gtk::gdk_pixbuf::InterpType::Bilinear).unwrap_or(sq);
                Some(to_rgba(&sq))
            }).collect::<Vec<_>>()
        }, move |thumbs: Vec<Option<Rgba>>| {
            for (pic, t) in pics.iter().zip(thumbs) {
                if let Some(t) = t {
                    pic.set_paintable(Some(&gdk::Texture::for_pixbuf(&from_rgba(&t))));
                }
            }
        });
    }

    fn mark_strip(self: &Rc<Self>) {
        let cur = self.current.get();
        let mut c = self.strip.first_child();
        while let Some(w) = c {
            let i = unsafe { w.data::<usize>("indice").map(|p| *p.as_ptr()) };
            if i == Some(cur) {
                w.add_css_class("atual");
            } else {
                w.remove_css_class("atual");
            }
            c = w.next_sibling();
        }
        // (the strip moves on when the one in view leaves the thumbnails read)
        let index = |w: Option<gtk::Widget>| w.and_then(|w| unsafe { w.data::<usize>("indice").map(|p| *p.as_ptr()) });
        if let (Some(first), Some(last)) = (index(self.strip.first_child()), index(self.strip.last_child())) {
            if cur < first || cur > last {
                self.fill_strip();
            }
        }
    }

    // ------------------------------------------------------------------------------------------- drawing

    /// The fit's zoom for this canvas (never above 100 % for a small picture).
    fn fit_zoom(&self, w: f64, h: f64) -> f64 {
        let Some(img) = self.image.borrow().as_ref().map(|i| (i.width as f64, i.height as f64)) else { return 1.0 };
        ((w - 80.0) / img.0).min((h - 80.0) / img.1).min(1.0).max(0.01)
    }

    fn current_zoom(&self) -> f64 {
        let (w, h) = (f64::from(self.canvas.width()), f64::from(self.canvas.height()));
        self.zoom.get().unwrap_or_else(|| self.fit_zoom(w, h))
    }

    /// Where the picture's top-left corner is, and its zoom.
    fn placement(&self, w: f64, h: f64) -> Option<(f64, f64, f64)> {
        let img = self.image.borrow();
        let img = img.as_ref()?;
        let z = self.zoom.get().unwrap_or_else(|| self.fit_zoom(w, h));
        let (px, py) = self.pan.get();
        let x = (w - img.width as f64 * z) / 2.0 + px + self.slide.get() * 90.0;
        let y = (h - img.height as f64 * z) / 2.0 + py;
        Some((x, y, z))
    }

    /// A point of the canvas in the picture's pixels.
    fn to_image(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let (w, h) = (f64::from(self.canvas.width()), f64::from(self.canvas.height()));
        let (ox, oy, z) = self.placement(w, h)?;
        let img = self.image.borrow();
        let img = img.as_ref()?;
        Some((((x - ox) / z).clamp(0.0, img.width as f64), ((y - oy) / z).clamp(0.0, img.height as f64)))
    }

    fn draw(&self, cr: &cairo::Context, w: f64, h: f64) {
        cr.set_source_rgb(0x10 as f64 / 255.0, 0x11 as f64 / 255.0, 0x14 as f64 / 255.0);
        let _ = cr.paint();
        let Some((x, y, z)) = self.placement(w, h) else { return };
        let Some(surf) = self.surface.borrow().clone() else { return };
        let (iw, ih) = (f64::from(surf.width()), f64::from(surf.height()));
        // a transparent picture shows over a soft checkerboard
        cr.save().ok();
        cr.rectangle(x, y, iw * z, ih * z);
        cr.clip();
        let cell = 10.0;
        for (i, yy) in (0..((ih * z / cell).ceil() as i64)).map(|k| (k, y + k as f64 * cell)) {
            for j in 0..((iw * z / cell).ceil() as i64) {
                let c = if (i + j) % 2 == 0 { 0.11 } else { 0.14 };
                cr.set_source_rgb(c, c, c + 0.01);
                cr.rectangle(x + j as f64 * cell, yy, cell, cell);
                let _ = cr.fill();
            }
        }
        cr.restore().ok();
        let paint = |s: &cairo::ImageSurface| {
            cr.save().ok();
            cr.translate(x, y);
            cr.scale(z, z);
            let _ = cr.set_source_surface(s, 0.0, 0.0);
            cr.source().set_filter(if z < 1.0 { cairo::Filter::Good } else { cairo::Filter::Fast });
            let _ = cr.paint_with_alpha(1.0 - self.slide.get().abs() * 0.6);
            cr.restore().ok();
        };
        paint(&surf);
        // an edit waiting: after it to the right of the curtain, before it to the left
        if let Some(p) = self.pending.borrow().as_ref() {
            if let Some(s) = &p.surface {
                let cx = w * self.curtain.get();
                cr.save().ok();
                cr.rectangle(cx, 0.0, w - cx, h);
                cr.clip();
                cr.set_source_rgb(0x10 as f64 / 255.0, 0x11 as f64 / 255.0, 0x14 as f64 / 255.0);
                cr.rectangle(x, y, iw * z, ih * z);
                let _ = cr.fill();
                // (the background removed: the checkerboard shows through)
                for i in 0..((ih * z / cell).ceil() as i64) {
                    for j in 0..((iw * z / cell).ceil() as i64) {
                        let c = if (i + j) % 2 == 0 { 0.11 } else { 0.14 };
                        cr.set_source_rgb(c, c, c + 0.01);
                        cr.rectangle(x + j as f64 * cell, y + i as f64 * cell, cell, cell);
                        let _ = cr.fill();
                    }
                }
                paint(s);
                cr.restore().ok();
                // the curtain's line and its handle
                cr.set_source_rgb(0xA5 as f64 / 255.0, 0xA1 as f64 / 255.0, 0xE2 as f64 / 255.0);
                cr.rectangle(cx - 1.0, 0.0, 2.0, h);
                let _ = cr.fill();
                cr.arc(cx, h / 2.0, 14.0, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
                cr.set_source_rgb(0x1C as f64 / 255.0, 0x1D as f64 / 255.0, 0x24 as f64 / 255.0);
                for d in [-5.0, 5.0] {
                    cr.move_to(cx + d, h / 2.0 - 5.0);
                    cr.line_to(cx + d + d.signum() * 4.0, h / 2.0);
                    cr.line_to(cx + d, h / 2.0 + 5.0);
                }
                cr.set_line_width(2.0);
                let _ = cr.stroke();
                // before / after
                cr.select_font_face("Inter", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
                cr.set_font_size(11.0);
                cr.set_source_rgba(0.89, 0.89, 0.9, 0.85);
                cr.move_to(cx - 64.0, 26.0);
                let _ = cr.show_text(tr("BEFORE"));
                cr.move_to(cx + 18.0, 26.0);
                let _ = cr.show_text(tr("AFTER"));
            }
        }
        // cropping: the outside darker, the rectangle outlined
        if self.mode.get() == Mode::Crop {
            if let Some((ax, ay, bx, by)) = self.crop.get() {
                let (l, t) = (x + ax.min(bx) * z, y + ay.min(by) * z);
                let (rw, rh) = ((ax - bx).abs() * z, (ay - by).abs() * z);
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.55);
                cr.set_fill_rule(cairo::FillRule::EvenOdd);
                cr.rectangle(0.0, 0.0, w, h);
                cr.rectangle(l, t, rw, rh);
                let _ = cr.fill();
                cr.set_fill_rule(cairo::FillRule::Winding);
                cr.set_source_rgb(0xA5 as f64 / 255.0, 0xA1 as f64 / 255.0, 0xE2 as f64 / 255.0);
                cr.set_line_width(2.0);
                cr.rectangle(l, t, rw, rh);
                let _ = cr.stroke();
            }
        }
    }

    // ------------------------------------------------------------------------------------------- zoom

    fn update_zoom_label(&self) {
        self.zoom_label.set_text(&format!("{:.0}%", self.current_zoom() * 100.0));
    }

    fn set_zoom(&self, z: Option<f64>) {
        self.zoom.set(z.map(|z| z.clamp(0.02, 16.0)));
        if z.is_none() {
            self.pan.set((0.0, 0.0));
        }
        self.update_zoom_label();
        self.canvas.queue_draw();
        if debug() {
            eprintln!("debug imagens: zoom {}", self.zoom_label.text());
        }
    }

    /// Zooms by a factor, keeping the point under the pointer where it is.
    fn zoom_by(&self, f: f64, at: Option<(f64, f64)>) {
        let (w, h) = (f64::from(self.canvas.width()), f64::from(self.canvas.height()));
        let old = self.current_zoom();
        let new = (old * f).clamp(0.02, 16.0);
        if let (Some((ax, ay)), Some((ox, oy, _))) = (at, self.placement(w, h)) {
            // the picture's point under the pointer stays under it
            let (ix, iy) = ((ax - ox) / old, (ay - oy) / old);
            let img = self.image.borrow();
            if let Some(img) = img.as_ref() {
                let nx = ax - ix * new - (w - img.width as f64 * new) / 2.0;
                let ny = ay - iy * new - (h - img.height as f64 * new) / 2.0;
                self.pan.set((nx, ny));
            }
        }
        self.zoom.set(Some(new));
        self.update_zoom_label();
        self.canvas.queue_draw();
    }

    fn toggle_zoom(&self, at: Option<(f64, f64)>) {
        if self.zoom.get().is_some() {
            self.set_zoom(None);
        } else {
            let f = 1.0 / self.current_zoom();
            self.zoom_by(f, at);
        }
    }

    // ------------------------------------------------------------------------------------------- edits

    /// An edit to see before keeping: the curtain shows before and after.
    fn propose(self: &Rc<Self>, what: &str, label: &str, after: Rgba) {
        let surface = surface_of(&after);
        *self.pending.borrow_mut() = Some(Pending { what: what.to_string(), after, surface });
        self.curtain.set(0.5);
        self.edit_label.set_text(label);
        self.edit_bar.set_visible(true);
        self.show_tools();
        self.canvas.queue_draw();
    }

    fn discard_quietly(&self) {
        self.pending.borrow_mut().take();
        self.edit_bar.set_visible(false);
    }

    fn discard(self: &Rc<Self>) {
        self.discard_quietly();
        self.canvas.queue_draw();
        if debug() {
            eprintln!("debug imagens: edição descartada");
        }
    }

    /// The edit is kept as a copy next to the picture (a PNG, so a transparent background stays), and shown.
    fn keep(self: &Rc<Self>) {
        let (Some(p), Some(pending)) = (self.path(), self.pending.borrow_mut().take()) else { return };
        self.edit_bar.set_visible(false);
        let dest = copy_path(&p, &pending.what);
        match from_rgba(&pending.after).savev(&dest, "png", &[]) {
            Ok(()) => {
                if debug() {
                    eprintln!("debug imagens: cópia salva «{}»", dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                }
                self.show_file(&dest);
            }
            Err(e) => self.say(&trf("The copy could not be saved: {error}", &[("error", e.to_string())])),
        }
    }

    /// Runs an edit on the picture's pixels in the background.
    fn edit(self: &Rc<Self>, what: &'static str, label: String, f: impl FnOnce(Rgba) -> (Rgba, String) + Send + 'static) {
        let Some(img) = self.image.borrow().clone() else { return };
        if self.busy.replace(true) {
            return;
        }
        self.edit_label.set_text(tr("Working…"));
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || f(img), move |(after, note): (Rgba, String)| {
            let Some(v) = w.upgrade() else { return };
            v.busy.set(false);
            if debug() {
                eprintln!("debug imagens: {what} pronto ({note})");
            }
            v.propose(what, &label, after);
        });
    }

    fn rotate(self: &Rc<Self>) {
        let Some(p) = self.path() else { return };
        if self.busy.get() {
            return;
        }
        // (saved in its own format when the system can write it; else a PNG copy)
        let ext = p.extension().and_then(|e| e.to_str()).map(str::to_lowercase).unwrap_or_default();
        let kind = match ext.as_str() {
            "png" => Some("png"),
            "jpg" | "jpeg" => Some("jpeg"),
            "bmp" => Some("bmp"),
            "tif" | "tiff" => Some("tiff"),
            _ => None,
        };
        let Some(img) = self.image.borrow().clone() else { return };
        let pb = from_rgba(&img).rotate_simple(PixbufRotation::Clockwise).unwrap_or_else(|| from_rgba(&img));
        let (dest, res) = match kind {
            Some(k) => {
                let opts: &[(&str, &str)] = if k == "jpeg" { &[("quality", "92")] } else { &[] };
                // (a JPEG has no transparency: the alpha the viewer added goes)
                let pb = if k == "jpeg" { flatten(&to_rgba(&pb)) } else { pb };
                (p.clone(), pb.savev(&p, k, opts))
            }
            None => {
                let d = copy_path(&p, tr("rotated"));
                (d.clone(), pb.savev(&d, "png", &[]))
            }
        };
        match res {
            Ok(()) => {
                if debug() {
                    eprintln!("debug imagens: girada e salva «{}»", dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                }
                self.show_file(&dest);
            }
            Err(e) => self.say(&trf("The picture could not be saved: {error}", &[("error", e.to_string())])),
        }
    }

    fn trash(self: &Rc<Self>) {
        let Some(p) = self.path() else { return };
        match gio::File::for_path(&p).trash(gio::Cancellable::NONE) {
            Ok(()) => {
                if debug() {
                    eprintln!("debug imagens: para a lixeira «{}»", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                }
                let i = self.current.get();
                self.files.borrow_mut().retain(|f| f != &p);
                let n = self.files.borrow().len();
                if n == 0 {
                    self.fill_strip();
                    self.show_empty();
                    return;
                }
                self.current.set(i.min(n - 1));
                self.fill_strip();
                self.load_current(1.0);
                self.say(&trf("{name} went to the trash (Files › Trash brings it back)", &[("name", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())]));
            }
            Err(e) => self.say(&e.to_string()),
        }
    }

    fn start_crop(self: &Rc<Self>) {
        if self.image.borrow().is_none() {
            return;
        }
        self.discard_quietly();
        self.mode.set(Mode::Crop);
        // (most of the picture to begin with)
        if let Some(img) = self.image.borrow().as_ref() {
            let (w, h) = (img.width as f64, img.height as f64);
            self.crop.set(Some((w * 0.1, h * 0.1, w * 0.9, h * 0.9)));
        }
        self.crop_bar.set_visible(true);
        self.canvas.queue_draw();
    }

    fn end_crop(self: &Rc<Self>) {
        self.mode.set(Mode::View);
        self.crop.set(None);
        self.crop_bar.set_visible(false);
        self.canvas.queue_draw();
    }

    fn apply_crop(self: &Rc<Self>) {
        let (Some((ax, ay, bx, by)), Some(img)) = (self.crop.get(), self.image.borrow().clone()) else { return };
        let (l, t) = (ax.min(bx).round() as usize, ay.min(by).round() as usize);
        let (cw, ch) = (((ax - bx).abs().round() as usize).max(1), ((ay - by).abs().round() as usize).max(1));
        let (cw, ch) = (cw.min(img.width - l.min(img.width - 1)), ch.min(img.height - t.min(img.height - 1)));
        let mut data = Vec::with_capacity(cw * ch * 4);
        for y in t..t + ch {
            let s = (y * img.width + l) * 4;
            data.extend_from_slice(&img.data[s..s + cw * 4]);
        }
        self.end_crop();
        self.propose(tr("cropped"), &trf("Cropped to {w} × {h}", &[("w", cw.to_string()), ("h", ch.to_string())]), Rgba { width: cw, height: ch, data });
        // (a crop changes the size: the curtain makes no sense, the result is shown whole)
        self.curtain.set(0.0);
        self.canvas.queue_draw();
    }

    // ------------------------------------------------------------------------------------------- the agent's actions

    fn action(self: &Rc<Self>, id: &str) {
        if self.image.borrow().is_none() || self.busy.get() {
            return;
        }
        if debug() {
            eprintln!("debug imagens: ação {id}");
        }
        match id {
            "fundo" => self.edit("sem fundo", tr("Without the background — drag the curtain to compare").to_string(), |mut img| {
                let n = image_ops::remove_background(&mut img, 32.0);
                let note = format!("{n} px transparentes");
                (img, note)
            }),
            "nitidez" => self.edit("nítida", tr("Sharper — drag the curtain to compare").to_string(), |img| (image_ops::sharpen(&img, 0.9), String::new())),
            "variacoes" => self.variations(),
            "descrever" => self.ask_model(tr("Describe this picture in two or three short sentences: what it shows, its style and anything notable. Answer in Brazilian Portuguese."), false),
            "texto" => self.ask_model(tr("Transcribe all the text in this image exactly as written, line by line, keeping the original language. Answer only with the text; if there is no text, answer with nothing."), true),
            _ => {}
        }
    }

    fn variations(self: &Rc<Self>) {
        let Some(img) = self.image.borrow().clone() else { return };
        self.busy.set(true);
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            // (worked on a smaller copy for the thumbnails, on the whole picture for the one chosen)
            image_ops::variations(&img).into_iter().map(|(n, r)| (n, r)).collect::<Vec<_>>()
        }, move |vs: Vec<(&'static str, Rgba)>| {
            let Some(v) = w.upgrade() else { return };
            v.busy.set(false);
            v.clear_result();
            v.result.append(&rotulo_left(tr("VARIATIONS"), "img-lado-titulo"));
            let grid = gtk::FlowBox::new();
            grid.set_selection_mode(gtk::SelectionMode::None);
            grid.set_max_children_per_line(2);
            grid.set_min_children_per_line(2);
            grid.set_column_spacing(6);
            grid.set_row_spacing(6);
            for (name, r) in vs {
                let b = gtk::Button::new();
                b.add_css_class("img-variacao");
                let col = caixa(Orientation::Vertical, 4, "");
                let pic = gtk::Picture::for_paintable(&gdk::Texture::for_pixbuf(&from_rgba(&r)));
                pic.set_size_request(104, 70);
                pic.set_content_fit(gtk::ContentFit::Cover);
                col.append(&pic);
                let label = match name { "quente" => tr("warm"), "fria" => tr("cool"), "pb" => tr("black and white"), _ => tr("vivid") };
                col.append(&rotulo(label, "img-detalhe-nome"));
                b.set_child(Some(&col));
                crate::central::posicao_debug_sempre(&format!("imagens variacao {name}"), b.upcast_ref());
                let w2 = Rc::downgrade(&v);
                b.connect_clicked(move |_| {
                    if let Some(v) = w2.upgrade() {
                        v.propose(&format!("{} {label}", tr("variation")), &trf("Variation: {name} — drag the curtain to compare", &[("name", label.to_string())]), r.clone());
                    }
                });
                grid.insert(&b, -1);
            }
            v.result.append(&grid);
            v.result.set_visible(true);
            if debug() {
                eprintln!("debug imagens: 4 variações");
            }
        });
    }

    fn clear_result(&self) {
        while let Some(c) = self.result.first_child() {
            self.result.remove(&c);
        }
    }

    /// The local vision model looks at the picture (a smaller PNG of it): a description, or the text it has.
    fn ask_model(self: &Rc<Self>, question: &str, copy: bool) {
        let Some(img) = self.image.borrow().clone() else { return };
        self.busy.set(true);
        self.clear_result();
        self.result.append(&rotulo_left(if copy { tr("TEXT IN THE PICTURE") } else { tr("DESCRIPTION") }, "img-lado-titulo"));
        let wait = rotulo_left(tr("The local model is looking at the picture…"), "img-detalhe-nome");
        wait.set_wrap(true);
        // (it wraps inside the panel's 250 px instead of widening it)
        wait.set_max_width_chars(1);
        self.result.append(&wait);
        self.result.set_visible(true);
        let question = question.to_string();
        let bin = crate::voz::agent_binary();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            // (at most 1280 px on its longer side: enough to read, and quicker)
            let pb = from_rgba(&img);
            let scale = (1280.0 / img.width.max(img.height) as f64).min(1.0);
            let pb = if scale < 1.0 { pb.scale_simple((img.width as f64 * scale) as i32, (img.height as f64 * scale) as i32, gtk::gdk_pixbuf::InterpType::Bilinear).unwrap_or(pb) } else { pb };
            let tmp = std::env::temp_dir().join(format!("kalango-imagem-{}.png", std::process::id()));
            pb.savev(&tmp, "png", &[]).map_err(|e| e.to_string())?;
            let o = std::process::Command::new(&bin).args(["--json", "ver"]).arg(&tmp).arg(&question).stdin(std::process::Stdio::null()).output();
            let _ = std::fs::remove_file(&tmp);
            let o = o.map_err(|e| e.to_string())?;
            let v: serde_json::Value = String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| serde_json::from_str(l).ok()).last().unwrap_or_default();
            match v["texto"].as_str() {
                Some(t) if v["tipo"] == "visao" => Ok(t.trim().to_string()),
                _ => Err(v["texto"].as_str().map(str::to_string).unwrap_or_else(|| String::from_utf8_lossy(&o.stderr).trim().lines().last().unwrap_or("").to_string())),
            }
        }, move |r: Result<String, String>| {
            let Some(v) = w.upgrade() else { return };
            v.busy.set(false);
            match r {
                Ok(t) if copy && t.trim().is_empty() => wait.set_text(tr("The model found no text in the picture.")),
                Ok(t) => {
                    wait.set_text(&t);
                    wait.set_selectable(true);
                    wait.remove_css_class("img-detalhe-nome");
                    wait.add_css_class("img-texto");
                    if copy {
                        v.window.clipboard().set_text(&t);
                        let c = rotulo_left(tr("Copied: Ctrl+V puts it where you want."), "img-detalhe-nome");
                        c.set_wrap(true);
                        c.set_max_width_chars(1);
                        v.result.append(&c);
                    }
                    if debug() {
                        eprintln!("debug imagens: {} ({} caracteres)", if copy { "texto copiado" } else { "descrição" }, t.chars().count());
                    }
                }
                Err(e) => wait.set_text(&trf("The local model could not look at it: {error}", &[("error", e)])),
            }
        });
    }

    // ------------------------------------------------------------------------------------------- menus

    fn adjust_popover(self: &Rc<Self>, b: &gtk::MenuButton) {
        let pop = gtk::Popover::new();
        pop.add_css_class("menu");
        let col = caixa(Orientation::Vertical, 8, "img-ajustes");
        col.set_size_request(240, -1);
        let mut scales = Vec::new();
        for (name, lo, hi, val) in [(tr("Brightness"), -0.5, 0.5, 0.0), (tr("Contrast"), -0.5, 0.8, 0.0), (tr("Colour"), 0.0, 2.0, 1.0)] {
            col.append(&rotulo_left(name, "img-detalhe-nome"));
            let s = gtk::Scale::with_range(Orientation::Horizontal, lo, hi, 0.01);
            s.add_css_class("cfg-escala");
            s.set_draw_value(false);
            s.set_value(val);
            col.append(&s);
            scales.push(s);
        }
        pop.set_child(Some(&col));
        b.set_popover(Some(&pop));
        let scales = Rc::new(scales);
        let timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        for s in scales.iter() {
            let (w, scales, timer) = (Rc::downgrade(self), scales.clone(), timer.clone());
            s.connect_value_changed(move |_| {
                if let Some(id) = timer.borrow_mut().take() {
                    id.remove();
                }
                let (w, scales, t2) = (w.clone(), scales.clone(), timer.clone());
                let id = glib::timeout_add_local_once(Duration::from_millis(120), move || {
                    t2.borrow_mut().take();
                    let Some(v) = w.upgrade() else { return };
                    let (b, c, s) = (scales[0].value(), scales[1].value(), scales[2].value());
                    v.adjust.set((b, c, s));
                    v.busy.set(false);
                    v.edit("ajustada", tr("Adjusted — drag the curtain to compare").to_string(), move |img| (image_ops::adjust(&img, b, c, s), String::new()));
                });
                *timer.borrow_mut() = Some(id);
            });
        }
        // (opened again: back to where the picture is)
        let scales2 = scales.clone();
        pop.connect_show(move |_| {
            for (s, v) in scales2.iter().zip([0.0, 0.0, 1.0]) {
                s.set_value(v);
            }
        });
    }

    fn share_popover(self: &Rc<Self>, b: &gtk::MenuButton) {
        let pop = gtk::Popover::new();
        pop.add_css_class("menu");
        let col = caixa(Orientation::Vertical, 2, "menu-lista");
        let entries: [(&str, &str, Box<dyn Fn(&Rc<ImageViewer>)>); 3] = [
            ("copiar", tr("Copy the picture"), Box::new(|v: &Rc<ImageViewer>| {
                if let Some(img) = v.image.borrow().as_ref() {
                    v.window.clipboard().set_texture(&gdk::Texture::for_pixbuf(&from_rgba(img)));
                    v.say(tr("The picture is on the clipboard."));
                }
            })),
            ("caminho", tr("Copy the path"), Box::new(|v: &Rc<ImageViewer>| {
                if let Some(p) = v.path() {
                    v.window.clipboard().set_text(&p.display().to_string());
                    v.say(tr("The path is on the clipboard."));
                }
            })),
            ("pasta", tr("Show in the folder"), Box::new(|v: &Rc<ImageViewer>| {
                if let (Some(p), Some(s)) = (v.path(), v.shell.upgrade()) {
                    s.abrir_arquivos(Some(&p));
                }
            })),
        ];
        for (id, label, f) in entries {
            let item = gtk::Button::with_label(label);
            item.add_css_class("menu-item");
            if let Some(l) = item.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            let (w, pop2) = (Rc::downgrade(self), pop.clone());
            item.connect_clicked(move |_| {
                pop2.popdown();
                if let Some(v) = w.upgrade() {
                    f(&v);
                }
                if debug() {
                    eprintln!("debug imagens: compartilhar {id}");
                }
            });
            col.append(&item);
        }
        pop.set_child(Some(&col));
        b.set_popover(Some(&pop));
    }

    fn menu(self: &Rc<Self>, items: &gtk::Box, pop: &gtk::Popover) {
        let add = |label: &str, f: Box<dyn Fn(&Rc<ImageViewer>)>| {
            let b = gtk::Button::with_label(label);
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            b.add_css_class("menu-item");
            let (w, pop) = (Rc::downgrade(self), pop.clone());
            b.connect_clicked(move |_| {
                pop.popdown();
                if let Some(v) = w.upgrade() {
                    f(&v);
                }
            });
            items.append(&b);
        };
        add(tr("Open…"), Box::new(|v| v.choose()));
        add(tr("Use as wallpaper"), Box::new(|v| {
            if let Some(p) = v.path() {
                let mut t = crate::tema::ler();
                let s = p.display().to_string();
                t.papeis.retain(|x| *x != s);
                t.papeis.insert(0, s.clone());
                t.papeis.truncate(5);
                t.papel = Some(s);
                crate::tema::salvar(&t);
                crate::tema::aplicar(&t);
                v.say(tr("It is the wallpaper now."));
                if debug() {
                    eprintln!("debug imagens: papel de parede «{}»", p.display());
                }
            }
        }));
        add(tr("Show in the folder"), Box::new(|v| {
            if let (Some(p), Some(s)) = (v.path(), v.shell.upgrade()) {
                s.abrir_arquivos(Some(&p));
            }
        }));
    }

    /// A short line over the picture (what happened), gone after 4 s.
    fn say(self: &Rc<Self>, text: &str) {
        self.edit_label.set_text(text);
        self.edit_bar.set_visible(true);
        let keep_buttons = self.pending.borrow().is_some();
        let mut c = self.edit_bar.first_child().and_then(|f| f.next_sibling());
        while let Some(b) = c {
            b.set_visible(keep_buttons);
            c = b.next_sibling();
        }
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_secs(4), move || {
            if let Some(v) = w.upgrade() {
                if v.pending.borrow().is_none() {
                    v.edit_bar.set_visible(false);
                }
                let mut c = v.edit_bar.first_child().and_then(|f| f.next_sibling());
                while let Some(b) = c {
                    b.set_visible(true);
                    c = b.next_sibling();
                }
            }
        });
    }
}

/// A picture without transparency (a JPEG): what was transparent becomes the viewer's background.
fn flatten(r: &Rgba) -> Pixbuf {
    let mut data = Vec::with_capacity(r.width * r.height * 3);
    for p in r.data.chunks_exact(4) {
        let a = p[3] as f64 / 255.0;
        for (c, bg) in p[..3].iter().zip([0x10u8, 0x11, 0x14]) {
            data.push((*c as f64 * a + bg as f64 * (1.0 - a)).round() as u8);
        }
    }
    Pixbuf::from_bytes(&glib::Bytes::from(&data[..]), Colorspace::Rgb, false, 8, r.width as i32, r.height as i32, (r.width * 3) as i32)
}

fn rotulo_left(t: &str, class: &str) -> gtk::Label {
    let l = rotulo(t, class);
    l.set_xalign(0.0);
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_get_a_free_name_and_origins_are_read() {
        let dir = std::env::temp_dir().join(format!("kalango-img-teste-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("nave.png");
        std::fs::write(&p, b"\x89PNG\r\n\x1a\n....tEXtparameters\0a ship, Steps: 30").unwrap();
        assert_eq!(copy_path(&p, "sem fundo"), dir.join("nave (sem fundo).png"));
        std::fs::write(dir.join("nave (sem fundo).png"), b"x").unwrap();
        assert_eq!(copy_path(&p, "sem fundo"), dir.join("nave (sem fundo) 2.png"));
        assert_eq!(origin_of(&p), "made by AI");
        assert!(is_image(&p) && !is_image(&dir.join("nota.txt")));
        assert_eq!(folder_images(&dir).len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
