//! The screenshot tool (design "Captura de tela"; Print): the screen freezes under a dimmed veil and a bar at the
//! bottom — what (the whole screen, a window, an area that follows the pointer 1:1 with handles that grow on hover),
//! a photo or a video, the microphone and the pointer for a video, "Extrair texto" and "Capturar". A photo is saved in
//! Imagens/Capturas de tela and copied (a 90 ms flash); "Extrair texto" has the local vision model read the area and
//! copies the text; a video is recorded by niri (its ScreenCast) through GStreamer into Vídeos/Gravações de tela, the
//! island showing it until it is stopped (a click on it, or Print again).

use crate::conversa::ui_icon;
use crate::superficies::{caixa, camada, rotulo};
use gtk::prelude::*;
use gtk::{gdk, gio, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use kalango_movimento::{Animavel, Canal, RAPIDA};
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum What {
    Screen,
    Window,
    Area,
}

/// A rectangle on the screen, in logical pixels (x, y, w, h).
type Rect = (f64, f64, f64, f64);

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    None,
    New(f64, f64),
    Move(Rect),
    Resize(usize, Rect),
}

struct Tool {
    window: gtk::ApplicationWindow,
    picture: gtk::Picture,
    area: gtk::DrawingArea,
    bar: gtk::Box,
    flash: Animavel,
    shot: RefCell<Option<gdk_pixbuf::Pixbuf>>,
    screen: Cell<(f64, f64)>,
    what: Cell<What>,
    video: Cell<bool>,
    mic: Cell<bool>,
    pointer: Cell<bool>,
    sel: Cell<Rect>,
    windows: RefCell<Vec<(u64, Rect)>>,
    window_id: Cell<Option<u64>>,
    hover_handle: Cell<Option<usize>>,
    drag: Cell<Drag>,
    buttons: RefCell<Vec<(String, gtk::Button)>>,
}

use gtk::gdk_pixbuf;

thread_local! {
    static TOOL: RefCell<Option<Rc<Tool>>> = const { RefCell::new(None) };
}

fn handles(r: Rect) -> [(f64, f64); 8] {
    let (x, y, w, h) = r;
    [(x, y), (x + w / 2.0, y), (x + w, y), (x + w, y + h / 2.0), (x + w, y + h), (x + w / 2.0, y + h), (x, y + h), (x, y + h / 2.0)]
}

/// A rectangle from two corners, kept on the screen.
fn rect_from(a: (f64, f64), b: (f64, f64), screen: (f64, f64)) -> Rect {
    let (x0, x1) = (a.0.min(b.0).max(0.0), a.0.max(b.0).min(screen.0));
    let (y0, y1) = (a.1.min(b.1).max(0.0), a.1.max(b.1).min(screen.1));
    (x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0))
}

/// The windows in view and where they are (niri's layout).
fn windows_in_view() -> Vec<(u64, Rect)> {
    let Ok(v) = crate::servicos::niri_pedido(json!("Windows")) else { return vec![] };
    let mut out: Vec<(u64, Rect, bool)> = v["Windows"].as_array().into_iter().flatten().filter_map(|w| {
        let l = &w["layout"];
        let tp = l["tile_pos_in_workspace_view"].as_array()?;
        let off = l["window_offset_in_tile"].as_array();
        let sz = l["window_size"].as_array()?;
        let x = tp.first()?.as_f64()? + off.and_then(|o| o.first()?.as_f64()).unwrap_or(0.0);
        let y = tp.get(1)?.as_f64()? + off.and_then(|o| o.get(1)?.as_f64()).unwrap_or(0.0);
        Some((w["id"].as_u64()?, (x, y, sz.first()?.as_f64()?, sz.get(1)?.as_f64()?), w["is_focused"].as_bool().unwrap_or(false)))
    }).collect();
    // the focused one first (on top when they overlap)
    out.sort_by_key(|(_, _, f)| !*f);
    out.into_iter().map(|(i, r, _)| (i, r)).collect()
}

fn tool() -> Option<Rc<Tool>> {
    if let Some(t) = TOOL.with(|t| t.borrow().clone()) {
        return Some(t);
    }
    let app = gio::Application::default().and_downcast::<gtk::Application>()?;
    let window = camada(&app, "kalango-captura", Layer::Overlay, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
    window.add_css_class("captura-fundo");
    crate::superficies::take_keyboard(&window);
    window.set_exclusive_zone(-1);
    let over = gtk::Overlay::new();
    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Fill);
    picture.set_can_shrink(true);
    over.set_child(Some(&picture));
    let area = gtk::DrawingArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);
    over.add_overlay(&area);
    let bar = caixa(Orientation::Horizontal, 8, "captura-barra");
    bar.set_halign(Align::Center);
    bar.set_valign(Align::End);
    bar.set_margin_bottom(88);
    over.add_overlay(&bar);
    let branco = caixa(Orientation::Vertical, 0, "captura-flash");
    branco.set_can_target(false);
    let flash = Animavel::new(&branco);
    flash.set_can_target(false);
    flash.definir(Canal::Opacidade, 0.0);
    over.add_overlay(&flash);
    window.set_child(Some(&over));
    let t = Rc::new(Tool {
        window, picture, area, bar, flash, shot: RefCell::default(), screen: Cell::new((1920.0, 1080.0)), what: Cell::new(What::Area), video: Cell::new(false),
        mic: Cell::new(false), pointer: Cell::new(true), sel: Cell::new((0.0, 0.0, 0.0, 0.0)), windows: RefCell::default(), window_id: Cell::new(None),
        hover_handle: Cell::new(None), drag: Cell::new(Drag::None), buttons: RefCell::default(),
    });
    let w = Rc::downgrade(&t);
    t.area.set_draw_func(move |_, cr, _, _| {
        if let Some(t) = w.upgrade() {
            t.draw(cr);
        }
    });
    // the area follows the pointer 1:1: a new one, moved, or resized by a handle
    let drag = gtk::GestureDrag::new();
    let w = Rc::downgrade(&t);
    drag.connect_drag_begin(move |_, x, y| {
        let Some(t) = w.upgrade() else { return };
        match t.what.get() {
            What::Area => {
                let sel = t.sel.get();
                let alca = handles(sel).iter().position(|(hx, hy)| (hx - x).abs() <= 9.0 && (hy - y).abs() <= 9.0);
                let dentro = x >= sel.0 && x <= sel.0 + sel.2 && y >= sel.1 && y <= sel.1 + sel.3 && sel.2 > 1.0;
                t.drag.set(match (alca, dentro) {
                    (Some(i), _) => Drag::Resize(i, sel),
                    (None, true) => Drag::Move(sel),
                    _ => Drag::New(x, y),
                });
            }
            What::Window => t.pick_window(x, y),
            What::Screen => {}
        }
    });
    let w = Rc::downgrade(&t);
    drag.connect_drag_update(move |g, dx, dy| {
        let Some(t) = w.upgrade() else { return };
        let tela = t.screen.get();
        let novo = match t.drag.get() {
            Drag::New(x, y) => rect_from((x, y), (x + dx, y + dy), tela),
            Drag::Move((x, y, w2, h)) => ((x + dx).clamp(0.0, tela.0 - w2), (y + dy).clamp(0.0, tela.1 - h), w2, h),
            Drag::Resize(i, (x, y, w2, h)) => {
                let (mut x0, mut y0, mut x1, mut y1) = (x, y, x + w2, y + h);
                if matches!(i, 0 | 6 | 7) { x0 += dx; }
                if matches!(i, 2 | 3 | 4) { x1 += dx; }
                if matches!(i, 0 | 1 | 2) { y0 += dy; }
                if matches!(i, 4 | 5 | 6) { y1 += dy; }
                rect_from((x0, y0), (x1, y1), tela)
            }
            Drag::None => return,
        };
        let _ = g;
        t.sel.set(novo);
        t.area.queue_draw();
    });
    let w = Rc::downgrade(&t);
    drag.connect_drag_end(move |_, _, _| {
        if let Some(t) = w.upgrade() {
            t.drag.set(Drag::None);
            t.refresh_bar();
            if debug() {
                let s = t.sel.get();
                eprintln!("debug captura: área {:.0}×{:.0} em {:.0} {:.0}", s.2, s.3, s.0, s.1);
            }
        }
    });
    t.area.add_controller(drag);
    // the handles grow under the pointer; in Window, the window under it lights up
    let motion = gtk::EventControllerMotion::new();
    let w = Rc::downgrade(&t);
    motion.connect_motion(move |_, x, y| {
        let Some(t) = w.upgrade() else { return };
        match t.what.get() {
            What::Area => {
                let h = handles(t.sel.get()).iter().position(|(hx, hy)| (hx - x).abs() <= 9.0 && (hy - y).abs() <= 9.0);
                if h != t.hover_handle.get() {
                    t.hover_handle.set(h);
                    t.area.queue_draw();
                }
            }
            What::Window => t.pick_window(x, y),
            What::Screen => {}
        }
    });
    t.area.add_controller(motion);
    let keys = gtk::EventControllerKey::new();
    let w = Rc::downgrade(&t);
    keys.connect_key_pressed(move |_, k, _, _| {
        let Some(t) = w.upgrade() else { return glib::Propagation::Proceed };
        match k {
            gdk::Key::Escape => t.close(),
            gdk::Key::Return | gdk::Key::KP_Enter => t.capture(),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    t.window.add_controller(keys);
    TOOL.with(|x| *x.borrow_mut() = Some(t.clone()));
    Some(t)
}

impl Tool {
    fn pick_window(&self, x: f64, y: f64) {
        let achada = self.windows.borrow().iter().find(|(_, r)| x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3).cloned();
        if let Some((id, r)) = achada {
            if self.window_id.get() != Some(id) {
                self.window_id.set(Some(id));
                self.sel.set(r);
                self.area.queue_draw();
            }
        }
    }

    fn draw(&self, cr: &gtk::cairo::Context) {
        let (sw, sh) = self.screen.get();
        let (x, y, w, h) = self.sel.get();
        // the veil outside the chosen part
        let (vr, vg, vb) = crate::tema::rgb("#0B0C10");
        cr.set_source_rgba(vr, vg, vb, 0.55);
        cr.set_fill_rule(gtk::cairo::FillRule::EvenOdd);
        cr.rectangle(0.0, 0.0, sw, sh);
        if w > 1.0 && h > 1.0 {
            cr.rectangle(x, y, w, h);
        }
        let _ = cr.fill();
        if w <= 1.0 || h <= 1.0 {
            return;
        }
        let (ar, ag, ab) = crate::tema::rgb("#A5A1E2");
        cr.set_source_rgb(ar, ag, ab);
        cr.set_line_width(1.5);
        cr.set_dash(&[6.0, 4.0], 0.0);
        cr.rectangle(x + 0.75, y + 0.75, w - 1.5, h - 1.5);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
        // the handles (an area's), growing under the pointer
        if self.what.get() == What::Area {
            for (i, (hx, hy)) in handles((x, y, w, h)).iter().enumerate() {
                let lado = if self.hover_handle.get() == Some(i) { 14.0 } else { 10.0 };
                cr.set_source_rgb(1.0, 1.0, 1.0);
                cr.rectangle(hx - lado / 2.0, hy - lado / 2.0, lado, lado);
                let _ = cr.fill_preserve();
                cr.set_source_rgb(ar, ag, ab);
                cr.set_line_width(1.5);
                let _ = cr.stroke();
            }
        }
        // its size above it
        let texto = format!("{:.0} × {:.0}", w, h);
        cr.select_font_face("Inter", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
        cr.set_font_size(12.0);
        let ext = cr.text_extents(&texto).ok();
        let tw = ext.map(|e| e.width()).unwrap_or(60.0) + 16.0;
        let ly = if y > 30.0 { y - 26.0 } else { y + 6.0 };
        // (the design's accent pill)
        cr.set_source_rgb(ar, ag, ab);
        cr.new_sub_path();
        cr.arc(x + tw - 4.0, ly + 4.0, 4.0, -std::f64::consts::FRAC_PI_2, 0.0);
        cr.arc(x + tw - 4.0, ly + 15.0, 4.0, 0.0, std::f64::consts::FRAC_PI_2);
        cr.arc(x + 4.0, ly + 15.0, 4.0, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
        cr.arc(x + 4.0, ly + 4.0, 4.0, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
        cr.close_path();
        let _ = cr.fill();
        let (tr2, tg, tb) = crate::tema::rgb("#1C1D24");
        cr.set_source_rgb(tr2, tg, tb);
        cr.move_to(x + 8.0, ly + 14.0);
        let _ = cr.show_text(&texto);
    }

    fn button(self: &Rc<Self>, key: &str, glifo: &str, label: &str, class: &str, f: impl Fn(&Rc<Tool>) + 'static) -> gtk::Button {
        let b = gtk::Button::new();
        b.add_css_class(class);
        let r = caixa(Orientation::Horizontal, 7, "");
        if !glifo.is_empty() {
            r.append(&ui_icon(glifo, if class == "captura-capturar" { "#1C1D24" } else if glifo == "spark" { "#A5A1E2" } else { "#B9BAC4" }, 16));
        }
        if !label.is_empty() {
            r.append(&gtk::Label::new(Some(label)));
        }
        b.set_child(Some(&r));
        let w = Rc::downgrade(self);
        b.connect_clicked(move |_| if let Some(t) = w.upgrade() { f(&t) });
        self.buttons.borrow_mut().push((key.to_string(), b.clone()));
        if debug() {
            crate::central::posicao_debug(&format!("captura {key}"), b.upcast_ref());
        }
        b
    }

    fn build_bar(self: &Rc<Self>) {
        while let Some(c) = self.bar.first_child() {
            self.bar.remove(&c);
        }
        self.buttons.borrow_mut().clear();
        let o_que = caixa(Orientation::Horizontal, 2, "captura-grupo");
        o_que.append(&self.button("tela", "monitor", tr("Screen"), "captura-opcao", |t| t.set_what(What::Screen)));
        o_que.append(&self.button("janela", "painel-direita", tr("Window"), "captura-opcao", |t| t.set_what(What::Window)));
        o_que.append(&self.button("area", "recortar", tr("Area"), "captura-opcao", |t| t.set_what(What::Area)));
        self.bar.append(&o_que);
        let como = caixa(Orientation::Horizontal, 2, "captura-grupo");
        como.append(&self.button("foto", "camera", tr("Photo"), "captura-opcao", |t| t.set_video(false)));
        como.append(&self.button("video", "video-d", tr("Video"), "captura-opcao", |t| t.set_video(true)));
        self.bar.append(&como);
        // a video's: the microphone and the pointer
        let mic = self.button("microfone", "microfone", "", "captura-alternar", |t| {
            t.mic.set(!t.mic.get());
            t.refresh_bar();
        });
        mic.set_tooltip_text(Some(tr("Record the microphone")));
        self.bar.append(&mic);
        let ponteiro = self.button("ponteiro", "ponteiro", "", "captura-alternar", |t| {
            t.pointer.set(!t.pointer.get());
            t.refresh_bar();
        });
        ponteiro.set_tooltip_text(Some(tr("Show the pointer")));
        self.bar.append(&ponteiro);
        let sep = gtk::Separator::new(Orientation::Vertical);
        sep.add_css_class("captura-separador");
        self.bar.append(&sep);
        self.bar.append(&self.button("extrair", "spark", tr("Extract text"), "captura-extrair", |t| t.extract_text()));
        self.bar.append(&self.button("capturar", "", tr("Capture"), "captura-capturar", |t| t.capture()));
        let esc = self.button("esc", "", "Esc", "captura-esc", |t| t.close());
        esc.set_tooltip_text(Some(tr("Cancel")));
        self.bar.append(&esc);
        self.refresh_bar();
    }

    fn refresh_bar(&self) {
        let sel = self.sel.get();
        let tem = sel.2 > 1.0 && sel.3 > 1.0;
        for (k, b) in self.buttons.borrow().iter() {
            let ativo = match k.as_str() {
                "tela" => self.what.get() == What::Screen,
                "janela" => self.what.get() == What::Window,
                "area" => self.what.get() == What::Area,
                "foto" => !self.video.get(),
                "video" => self.video.get(),
                "microfone" => self.mic.get(),
                "ponteiro" => self.pointer.get(),
                _ => false,
            };
            if ativo { b.add_css_class("ativa") } else { b.remove_css_class("ativa") }
            // (all of the bar stays in its place: what does not apply to a photo or a video is only off)
            match k.as_str() {
                "microfone" | "ponteiro" => b.set_sensitive(self.video.get()),
                "extrair" => b.set_sensitive(!self.video.get() && tem),
                "capturar" => b.set_sensitive(tem),
                _ => {}
            }
        }
    }

    fn set_what(self: &Rc<Self>, w: What) {
        self.what.set(w);
        self.window_id.set(None);
        let tela = self.screen.get();
        match w {
            What::Screen => self.sel.set((0.0, 0.0, tela.0, tela.1)),
            What::Window => {
                // the focused window, until the pointer goes over another
                let primeira = self.windows.borrow().first().cloned();
                match primeira {
                    Some((id, r)) => {
                        self.window_id.set(Some(id));
                        self.sel.set(r);
                    }
                    None => self.sel.set((0.0, 0.0, 0.0, 0.0)),
                }
            }
            What::Area => {}
        }
        if debug() {
            eprintln!("debug captura: modo {w:?}");
        }
        self.area.queue_draw();
        self.refresh_bar();
    }

    fn set_video(self: &Rc<Self>, v: bool) {
        self.video.set(v);
        if debug() {
            eprintln!("debug captura: {}", if v { "vídeo" } else { "foto" });
        }
        self.refresh_bar();
    }

    /// The chosen part of the frozen picture (the picture is in the screen's pixels; the selection in logical ones).
    fn cropped(&self) -> Option<gdk_pixbuf::Pixbuf> {
        let pb = self.shot.borrow().clone()?;
        let (sw, _) = self.screen.get();
        let k = pb.width() as f64 / sw.max(1.0);
        let (x, y, w, h) = self.sel.get();
        let (px, py) = ((x * k).round() as i32, (y * k).round() as i32);
        let pw = ((w * k).round() as i32).min(pb.width() - px).max(1);
        let ph = ((h * k).round() as i32).min(pb.height() - py).max(1);
        Some(pb.new_subpixbuf(px, py, pw, ph))
    }

    fn capture(self: &Rc<Self>) {
        let sel = self.sel.get();
        if sel.2 <= 1.0 || sel.3 <= 1.0 {
            return;
        }
        if self.video.get() {
            let (what, window, mic, pointer) = (self.what.get(), self.window_id.get(), self.mic.get(), self.pointer.get());
            self.close();
            start_recording(what, window, sel, self.screen.get(), mic, pointer);
            return;
        }
        let Some(pb) = self.cropped() else { return };
        let dir = destination(glib::UserDirectory::Pictures, "Imagens").join(tr("Screenshots"));
        let _ = std::fs::create_dir_all(&dir);
        let agora = glib::DateTime::now_local().ok().and_then(|d| d.format("%Y-%m-%d %H-%M-%S").ok()).map(|s| s.to_string()).unwrap_or_default();
        let path = dir.join(format!("{}.png", trf("Screenshot from {when}", &[("when", agora)])));
        let salvo = pb.savev(&path, "png", &[]).is_ok();
        #[allow(deprecated)]
        let tex = gdk::Texture::for_pixbuf(&pb);
        if let Some(d) = gdk::Display::default() {
            d.clipboard().set_texture(&tex);
        }
        if debug() {
            eprintln!("debug captura: foto {} ({}x{}){}", path.display(), pb.width(), pb.height(), if salvo { "" } else { " — não salva" });
        }
        // the flash, then the tool goes
        self.flash.definir(Canal::Opacidade, 0.85);
        self.flash.ir(Canal::Opacidade, 0.0, RAPIDA);
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(90), move || {
            if let Some(t) = w.upgrade() {
                t.close();
            }
        });
        if salvo {
            notify_file(tr("Screenshot saved"), &path, tr("It is also copied: paste it anywhere."));
        }
    }

    /// The local vision model reads the chosen part; the text is copied.
    fn extract_text(self: &Rc<Self>) {
        let Some(pb) = self.cropped() else { return };
        // an area of one colour has nothing to read (and a small model would make something up)
        if is_blank(&pb) {
            self.close();
            if debug() {
                eprintln!("debug captura: área sem texto (uma cor só)");
            }
            if let Some(s) = crate::shell_atual() {
                s.notificacoes().notify_local(tr("Screenshot"), tr("No text found"), tr("The area has no text the model could read."));
            }
            return;
        }
        // small text is read better bigger
        let pb = if pb.height() < 160 || pb.width() < 480 {
            pb.scale_simple(pb.width() * 2, pb.height() * 2, gdk_pixbuf::InterpType::Bilinear).unwrap_or(pb)
        } else {
            pb
        };
        let png = crate::voz::private_dir().join(format!("texto-{}.png", std::process::id()));
        if pb.savev(&png, "png", &[]).is_err() {
            return;
        }
        self.close();
        if debug() {
            eprintln!("debug captura: lendo o texto ({}x{})", pb.width(), pb.height());
        }
        let id = crate::shell_atual().map(|s| s.notificacoes().notify_local_acoes(tr("Screenshot"), tr("Reading the text…"), tr("The local model is reading the area."), &[], |_| {})).unwrap_or(0);
        let bin = crate::voz::agent_binary();
        let pergunta = tr("Transcribe all the text in this image exactly as written, line by line, keeping the original language. Answer only with the text; if there is no text, answer with nothing.").to_string();
        crate::lateral::em_segundo_plano(move || {
            let o = std::process::Command::new(&bin).args(["--json", "ver"]).arg(&png).arg(&pergunta).stdin(std::process::Stdio::null()).output();
            let _ = std::fs::remove_file(&png);
            let o = o.map_err(|e| e.to_string())?;
            let v: serde_json::Value = String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| serde_json::from_str(l).ok()).last().unwrap_or_default();
            match v["texto"].as_str() {
                Some(t) if v["tipo"] == "visao" => Ok(without_preamble(t)),
                _ => Err(v["texto"].as_str().map(str::to_string).unwrap_or_else(|| String::from_utf8_lossy(&o.stderr).trim().to_string())),
            }
        }, move |r: Result<String, String>| {
            let Some(s) = crate::shell_atual() else { return };
            s.notificacoes().dispensar(id);
            match r {
                Ok(t) if !t.is_empty() => {
                    if let Some(d) = gdk::Display::default() {
                        d.clipboard().set_text(&t);
                    }
                    if debug() {
                        eprintln!("debug captura: texto extraído ({} caracteres): «{}»", t.chars().count(), t.chars().take(120).collect::<String>().replace('\n', " / "));
                    }
                    s.notificacoes().notify_local(tr("Screenshot"), tr("Text copied"), &t);
                }
                Ok(_) => s.notificacoes().notify_local(tr("Screenshot"), tr("No text found"), tr("The area has no text the model could read.")),
                Err(e) => {
                    if debug() {
                        eprintln!("debug captura: ler o texto falhou: {e}");
                    }
                    s.notificacoes().notify_local(tr("Screenshot"), tr("The text could not be read"), &e);
                }
            }
        });
    }

    fn close(&self) {
        self.window.set_keyboard_mode(KeyboardMode::None);
        self.window.set_visible(false);
        self.shot.borrow_mut().take();
        self.picture.set_paintable(None::<&gdk::Paintable>);
    }
}

/// Whether a picture is all one colour (its luminance barely varies).
fn is_blank(pb: &gdk_pixbuf::Pixbuf) -> bool {
    let (w, h, n, rs) = (pb.width() as usize, pb.height() as usize, pb.n_channels() as usize, pb.rowstride() as usize);
    let px = pb.read_pixel_bytes();
    let (mut soma, mut soma2, mut k) = (0f64, 0f64, 0f64);
    // (a sample is enough)
    for y in (0..h).step_by((h / 64).max(1)) {
        for x in (0..w).step_by((w / 64).max(1)) {
            let i = y * rs + x * n;
            let Some(p) = px.get(i..i + 3) else { continue };
            let l = 0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64;
            soma += l;
            soma2 += l * l;
            k += 1.0;
        }
    }
    if k == 0.0 {
        return true;
    }
    let media = soma / k;
    (soma2 / k - media * media).max(0.0).sqrt() < 3.0
}

/// The model's text without a "Here is the text:" line before it.
fn without_preamble(t: &str) -> String {
    let t = t.trim();
    let mut linhas = t.lines();
    let primeira = linhas.next().unwrap_or("").trim();
    let low = primeira.to_lowercase();
    let preambulo = primeira.ends_with(':') && ["aqui está", "aqui esta", "segue", "here is", "here's", "o texto", "the text"].iter().any(|p| low.starts_with(p));
    let resto = if preambulo { linhas.collect::<Vec<_>>().join("\n") } else { t.to_string() };
    resto.trim().trim_matches('"').trim().to_string()
}

/// The folders captures and recordings are saved in (the Shelf shows what is new there).
pub fn folders() -> [PathBuf; 2] {
    [destination(glib::UserDirectory::Pictures, "Imagens").join(tr("Screenshots")), destination(glib::UserDirectory::Videos, "Vídeos").join(tr("Screen recordings"))]
}

/// Where captures go: the person's Pictures or Videos (KALANGO_CAPTURA_DESTINO in the session test: never the real
/// home).
fn destination(d: glib::UserDirectory, fallback: &str) -> PathBuf {
    if let Some(t) = std::env::var_os("KALANGO_CAPTURA_DESTINO").filter(|v| !v.is_empty()) {
        return PathBuf::from(t).join(fallback);
    }
    glib::user_special_dir(d).unwrap_or_else(|| glib::home_dir().join(fallback))
}

/// A notice for a saved file: "Abrir" and "Mostrar na pasta".
fn notify_file(title: &str, path: &std::path::Path, body: &str) {
    let Some(s) = crate::shell_atual() else { return };
    let (p1, p2) = (path.to_path_buf(), path.parent().map(|p| p.to_path_buf()).unwrap_or_default());
    s.notificacoes().notify_local_acoes(tr("Screenshot"), title, &format!("{} · {body}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
        &[("abrir", tr("Open")), ("pasta", tr("Show in folder"))], move |k| {
            let alvo = if k == "abrir" { &p1 } else { &p2 };
            let _ = gio::AppInfo::launch_default_for_uri(&gio::File::for_path(alvo).uri(), gio::AppLaunchContext::NONE);
        });
}

/// Print: the tool opens over a frozen picture of the screen, for a photo (Shift+Print: for a video); while a video
/// records, either stops it.
pub fn open(video: bool) {
    if RECORDING.with(|r| r.borrow().is_some()) {
        stop_recording();
        return;
    }
    let Some(t) = tool() else { return };
    if t.window.is_visible() {
        return;
    }
    let png = crate::voz::private_dir().join(format!("captura-{}.png", std::process::id()));
    let _ = std::fs::remove_file(&png);
    // the compositor's picture is ours (no notice; the clipboard comes back)
    crate::descrever::mark_own_capture();
    if crate::servicos::niri_acao(json!({"ScreenshotScreen": {"write_to_disk": true, "show_pointer": false, "path": png.display().to_string()}})).is_err() {
        return;
    }
    let tela = crate::superficies::tamanho_da_saida().map(|(w, h)| (w as f64, h as f64)).unwrap_or((1920.0, 1080.0));
    let t0 = Instant::now();
    let w = Rc::downgrade(&t);
    glib::timeout_add_local(Duration::from_millis(40), move || {
        let Some(t) = w.upgrade() else { return glib::ControlFlow::Break };
        if !png.metadata().is_ok_and(|m| m.len() > 0) {
            return if t0.elapsed() > Duration::from_secs(3) { glib::ControlFlow::Break } else { glib::ControlFlow::Continue };
        }
        let Ok(pb) = gdk_pixbuf::Pixbuf::from_file(&png) else { return glib::ControlFlow::Continue };
        let _ = std::fs::remove_file(&png);
        crate::clipboard::repor_ultimo();
        #[allow(deprecated)]
        t.picture.set_paintable(Some(&gdk::Texture::for_pixbuf(&pb)));
        *t.shot.borrow_mut() = Some(pb);
        t.screen.set(tela);
        *t.windows.borrow_mut() = windows_in_view();
        // a photo, each time (or a video, by Shift+Print); the area in the middle (the last one, if it fits)
        t.video.set(video);
        let s = t.sel.get();
        if s.2 <= 1.0 || s.0 + s.2 > tela.0 || s.1 + s.3 > tela.1 || t.what.get() != What::Area {
            t.what.set(What::Area);
            t.sel.set(((tela.0 - 880.0).max(0.0) / 2.0, (tela.1 - 540.0).max(0.0) / 2.0 - 60.0, 880.0f64.min(tela.0), 540.0f64.min(tela.1)));
        }
        t.build_bar();
        t.window.present();
        crate::superficies::take_keyboard(&t.window);
        t.area.queue_draw();
        if debug() {
            eprintln!("debug captura: aberta ({}x{}, {} janelas à vista)", pb_size(&t).0, pb_size(&t).1, t.windows.borrow().len());
        }
        glib::ControlFlow::Break
    });
}

fn pb_size(t: &Tool) -> (i32, i32) {
    t.shot.borrow().as_ref().map(|p| (p.width(), p.height())).unwrap_or((0, 0))
}

// ---------------------------------------------------------------------------------------------------- video

struct Recording {
    child: std::process::Child,
    mutter: Option<String>,
    path: PathBuf,
    started: Instant,
}

thread_local! {
    static RECORDING: RefCell<Option<Recording>> = const { RefCell::new(None) };
    static ISLAND: RefCell<Option<(glib::WeakRef<gtk::Stack>, gtk::Label)>> = const { RefCell::new(None) };
}

/// Whether a video is being recorded.
pub fn recording() -> bool {
    RECORDING.with(|r| r.borrow().is_some())
}

/// The island while a video records: the red dot, the time and stop.
pub fn island(centro: &gtk::Stack) {
    let b = gtk::Button::new();
    b.add_css_class("ilha");
    b.add_css_class("gravando");
    b.set_valign(Align::Center);
    let r = caixa(Orientation::Horizontal, 8, "");
    let ponto = caixa(Orientation::Horizontal, 0, "gravando-ponto");
    ponto.set_size_request(8, 8);
    ponto.set_valign(Align::Center);
    r.append(&ponto);
    r.append(&rotulo(tr("Recording"), "data"));
    let tempo = rotulo("00:00", "hora");
    r.append(&tempo);
    let parar = caixa(Orientation::Horizontal, 0, "gravando-parar");
    parar.set_size_request(10, 10);
    parar.set_valign(Align::Center);
    r.append(&parar);
    b.set_child(Some(&r));
    b.set_tooltip_text(Some(tr("Stop recording")));
    b.connect_clicked(|_| stop_recording());
    if debug() {
        crate::central::posicao_debug("ilha gravando", b.upcast_ref());
    }
    centro.add_named(&b, Some("gravando"));
    ISLAND.with(|i| *i.borrow_mut() = Some((centro.downgrade(), tempo)));
}

fn show_island(on: bool) {
    ISLAND.with(|i| {
        if let Some((c, _)) = i.borrow().as_ref() {
            if let Some(c) = c.upgrade() {
                if on {
                    c.set_visible_child_name("gravando");
                } else if c.visible_child_name().as_deref() == Some("gravando") {
                    crate::focus_session::idle_island(&c);
                }
            }
        }
    });
}

fn video_failed(motivo: &str) {
    if debug() {
        eprintln!("debug captura: gravação não começou ({motivo})");
    }
    if let Some(s) = crate::shell_atual() {
        s.notificar(tr("Screenshot"), tr("The screen could not be recorded"), motivo);
    }
}

/// niri records the screen (or the window); GStreamer writes the video (the area cropped, the microphone mixed in).
fn start_recording(what: What, window: Option<u64>, sel: Rect, tela: (f64, f64), mic: bool, pointer: bool) {
    let Ok(c) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return };
    use crate::screen_share::{dict, mutter_call, MUTTER_PATH, MUTTER};
    let cursor = if pointer { 1u32 } else { 0u32 };
    let connector = crate::servicos::niri_pedido(json!("FocusedOutput")).ok().and_then(|v| v["FocusedOutput"]["name"].as_str().map(str::to_string)).unwrap_or_default();
    let c2 = c.clone();
    mutter_call(&c, MUTTER_PATH, MUTTER, "CreateSession", Some(glib::Variant::tuple_from_iter([dict(&[])])), Some("(o)"), move |r| {
        let Some(sessao) = r.ok().and_then(|v| v.child_value(0).get::<glib::variant::ObjectPath>().map(|p| p.as_str().to_string())) else {
            return video_failed(tr("The compositor does not record here."));
        };
        let (metodo, args) = match (what, window) {
            (What::Window, Some(id)) => ("RecordWindow", glib::Variant::tuple_from_iter([dict(&[("window-id", id.to_variant()), ("cursor-mode", cursor.to_variant())])])),
            _ => ("RecordMonitor", glib::Variant::tuple_from_iter([connector.to_variant(), dict(&[("cursor-mode", cursor.to_variant())])])),
        };
        let (c3, s3) = (c2.clone(), sessao.clone());
        mutter_call(&c2, &sessao, "org.gnome.Mutter.ScreenCast.Session", metodo, Some(args), Some("(o)"), move |r| {
            let Some(stream) = r.ok().and_then(|v| v.child_value(0).get::<glib::variant::ObjectPath>().map(|p| p.as_str().to_string())) else {
                return video_failed(metodo);
            };
            let esperando: Rc<Cell<Option<gio::SignalSubscriptionId>>> = Rc::default();
            let (c4, s4, e4) = (c3.clone(), s3.clone(), esperando.clone());
            let sub = c3.signal_subscribe(None, Some("org.gnome.Mutter.ScreenCast.Stream"), Some("PipeWireStreamAdded"), Some(&stream), None, gio::DBusSignalFlags::NONE,
                move |_, _, _, _, _, p| {
                    let Some(no) = p.child_value(0).get::<u32>() else { return };
                    if let Some(id) = e4.take() {
                        c4.signal_unsubscribe(id);
                    }
                    // the area: the screen's stream, cropped (its pixels are the screen's logical ones scaled)
                    let corte = (what == What::Area).then_some((sel, tela));
                    record_node(no, Some(s4.clone()), corte, mic);
                });
            esperando.set(Some(sub));
            mutter_call(&c3, &s3, "org.gnome.Mutter.ScreenCast.Session", "Start", None, None, |_| {});
            let (c5, e5, s5) = (c3.clone(), esperando, s3.clone());
            glib::timeout_add_local_once(Duration::from_secs(6), move || {
                if let Some(id) = e5.take() {
                    c5.signal_unsubscribe(id);
                    mutter_call(&c5, &s5, "org.gnome.Mutter.ScreenCast.Session", "Stop", None, None, |_| {});
                    video_failed(tr("The compositor did not give the stream."));
                }
            });
        });
    });
}

fn record_node(node: u32, mutter: Option<String>, crop: Option<(Rect, (f64, f64))>, mic: bool) {
    let dir = destination(glib::UserDirectory::Videos, "Vídeos").join(tr("Screen recordings"));
    let _ = std::fs::create_dir_all(&dir);
    let agora = glib::DateTime::now_local().ok().and_then(|d| d.format("%Y-%m-%d %H-%M-%S").ok()).map(|s| s.to_string()).unwrap_or_default();
    let path = dir.join(format!("{}.webm", trf("Screen recording from {when}", &[("when", agora)])));
    let mut args: Vec<String> = vec!["-e".into(), "pipewiresrc".into(), format!("path={node}"), "do-timestamp=true".into(), "!".into(), "videoconvert".into(), "!".into()];
    if let Some(((x, y, w, h), (sw, sh))) = crop {
        args.extend(["videoscale".into(), "!".into(), format!("video/x-raw,width={},height={}", sw as i32, sh as i32), "!".into()]);
        args.extend(["videocrop".into(), format!("left={}", x as i32), format!("top={}", y as i32), format!("right={}", (sw - x - w).max(0.0) as i32),
            format!("bottom={}", (sh - y - h).max(0.0) as i32), "!".into()]);
    }
    args.extend(["queue".into(), "!".into(), "vp8enc".into(), "deadline=1".into(), "cpu-used=8".into(), "!".into(), "webmmux".into(), "name=m".into(), "!".into(),
        "filesink".into(), format!("location={}", path.display())]);
    if mic {
        args.extend(["pulsesrc".into(), "!".into(), "audioconvert".into(), "!".into(), "opusenc".into(), "!".into(), "queue".into(), "!".into(), "m.".into()]);
    }
    let bin = std::env::var("KALANGO_GST").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "gst-launch-1.0".into());
    // (SIGINT reset: the shell ignores it, and -e needs it to finish the file)
    let mut cmd = std::process::Command::new(&bin);
    crate::voz::as_recorder(&mut cmd);
    let child = cmd.args(&args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
    match child {
        Ok(child) => {
            if debug() {
                eprintln!("debug captura: gravando (nó {node}{}{})", if crop.is_some() { ", área" } else { "" }, if mic { ", com o microfone" } else { "" });
            }
            RECORDING.with(|r| *r.borrow_mut() = Some(Recording { child, mutter, path, started: Instant::now() }));
            show_island(true);
            glib::timeout_add_local(Duration::from_millis(500), || {
                let s = RECORDING.with(|r| r.borrow().as_ref().map(|r| r.started.elapsed().as_secs()));
                let Some(s) = s else { return glib::ControlFlow::Break };
                ISLAND.with(|i| {
                    if let Some((_, l)) = i.borrow().as_ref() {
                        l.set_text(&format!("{:02}:{:02}", s / 60, s % 60));
                    }
                });
                glib::ControlFlow::Continue
            });
        }
        Err(e) => video_failed(&e.to_string()),
    }
}

/// Stops the video: GStreamer finishes the file, niri stops recording, a notice opens it.
pub fn stop_recording() {
    let Some(mut r) = RECORDING.with(|x| x.borrow_mut().take()) else { return };
    show_island(false);
    crate::voz::signal_child(r.child.id(), 2);
    if let (Some(m), Ok(c)) = (r.mutter.take(), gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)) {
        // (after GStreamer has its last frames)
        glib::timeout_add_local_once(Duration::from_millis(600), move || {
            crate::screen_share::mutter_call(&c, &m, "org.gnome.Mutter.ScreenCast.Session", "Stop", None, None, |_| {});
        });
    }
    let path = r.path.clone();
    crate::lateral::em_segundo_plano(move || {
        let t0 = Instant::now();
        loop {
            if let Ok(Some(_)) = r.child.try_wait() {
                break;
            }
            if t0.elapsed() > Duration::from_secs(8) {
                let _ = r.child.kill();
                let _ = r.child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }, move |_| {
        if debug() {
            eprintln!("debug captura: vídeo salvo {}", path.display());
        }
        if path.metadata().is_ok_and(|m| m.len() > 0) {
            notify_file(tr("Screen recording saved"), &path, tr("In Videos."));
        } else {
            video_failed(tr("The recording came out empty."));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_models_preamble_goes() {
        assert_eq!(without_preamble("Aqui está o texto da imagem:\n\n\"Reunião às 15h\""), "Reunião às 15h");
        assert_eq!(without_preamble("Reunião às 15h"), "Reunião às 15h");
        assert_eq!(without_preamble("Nota: comprar pão"), "Nota: comprar pão");
    }

    #[test]
    fn an_area_stays_on_the_screen_whichever_way_it_is_drawn() {
        assert_eq!(rect_from((100.0, 100.0), (50.0, 40.0), (1920.0, 1080.0)), (50.0, 40.0, 50.0, 60.0));
        assert_eq!(rect_from((1900.0, 10.0), (2100.0, -5.0), (1920.0, 1080.0)), (1900.0, 0.0, 20.0, 10.0));
        let h = handles((10.0, 20.0, 100.0, 50.0));
        assert_eq!(h[0], (10.0, 20.0));
        assert_eq!(h[4], (110.0, 70.0));
    }
}
