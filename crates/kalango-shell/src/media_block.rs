//! Media in the conversation (design "Chat · mídia (áudio, vídeo e imagens)"): files added to a message show as what
//! they are. Pictures in a row (one chosen: "Salvar em Imagens", "Usar como papel de parede", open it); a sound with
//! its play button, waveform and time, and "Transcrever" by the local audio model (the lines below it); a video with
//! its picture and length, opened in the Video app. Sound stays silent in the session test.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gstreamer as gst;
use gstreamer::prelude::*;
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

fn ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn is_image(p: &Path) -> bool {
    matches!(ext(p).as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp")
}

pub fn is_audio(p: &Path) -> bool {
    matches!(ext(p).as_str(), "mp3" | "m4a" | "ogg" | "opus" | "flac" | "wav" | "aac")
}

pub fn is_video(p: &Path) -> bool {
    matches!(ext(p).as_str(), "mp4" | "mkv" | "webm" | "mov" | "avi")
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

fn clock(s: f64) -> String {
    let s = s.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// A row of pictures; a click chooses one for the actions under them.
pub fn images(paths: &[PathBuf]) -> gtk::Widget {
    let col = caixa(Orientation::Vertical, 8, "midia-imagens");
    let row = caixa(Orientation::Horizontal, 8, "");
    row.set_homogeneous(true);
    let chosen = Rc::new(Cell::new(0usize));
    let mut frames = Vec::new();
    for (i, p) in paths.iter().take(4).enumerate() {
        let f = gtk::Button::new();
        f.add_css_class("midia-imagem");
        if i == 0 {
            f.add_css_class("ativo");
        }
        let pic = gtk::Picture::for_filename(p);
        pic.set_content_fit(gtk::ContentFit::Cover);
        pic.set_can_shrink(true);
        pic.set_size_request(-1, 160);
        f.set_child(Some(&pic));
        f.set_tooltip_text(Some(&p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
        row.append(&f);
        frames.push(f);
    }
    let frames = Rc::new(frames);
    for (i, f) in frames.iter().enumerate() {
        let (c, fs) = (chosen.clone(), frames.clone());
        f.connect_clicked(move |_| {
            c.set(i);
            for (k, x) in fs.iter().enumerate() {
                if k == i { x.add_css_class("ativo") } else { x.remove_css_class("ativo") }
            }
        });
    }
    col.append(&row);
    let acts = caixa(Orientation::Horizontal, 8, "");
    let paths: Rc<Vec<PathBuf>> = Rc::new(paths.to_vec());
    let botao = |glifo: &str, t: &str| {
        let b = gtk::Button::new();
        b.add_css_class("midia-acao");
        let c = caixa(Orientation::Horizontal, 6, "");
        c.append(&ui_icon(glifo, "#C9CAD1", 14));
        c.append(&rotulo(t, ""));
        b.set_child(Some(&c));
        acts.append(&b);
        b
    };
    let salvar = botao("download", tr("Save in Pictures"));
    let (ps, c) = (paths.clone(), chosen.clone());
    salvar.connect_clicked(move |b| {
        let Some(p) = ps.get(c.get()) else { return };
        let dir = crate::pastas::especial(glib::UserDirectory::Pictures, "Imagens");
        let _ = std::fs::create_dir_all(&dir);
        let nome = p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "imagem".into());
        let dest = crate::widgets::nome_livre(&dir, &nome, &ext(p));
        match std::fs::copy(p, &dest) {
            Ok(_) => {
                b.set_tooltip_text(Some(&trf("Saved in {folder}", &[("folder", dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())])));
                if debug() {
                    eprintln!("debug midia: imagem salva {}", dest.display());
                }
            }
            Err(e) => eprintln!("kalango-shell: salvar a imagem: {e}"),
        }
    });
    crate::central::posicao_debug("midia salvar imagem", salvar.upcast_ref());
    let papel = botao("imagem", tr("Use as wallpaper"));
    let (ps, c) = (paths.clone(), chosen.clone());
    papel.connect_clicked(move |_| {
        let Some(p) = ps.get(c.get()) else { return };
        let mut t = crate::tema::ler();
        let s = p.display().to_string();
        t.papeis.retain(|x| *x != s);
        t.papeis.insert(0, s.clone());
        t.papeis.truncate(5);
        t.papel = Some(s);
        crate::tema::salvar(&t);
        crate::tema::aplicar(&t);
        if debug() {
            eprintln!("debug midia: papel de parede {}", p.display());
        }
    });
    let abrir = botao("pasta", tr("Open"));
    let (ps, c) = (paths.clone(), chosen.clone());
    abrir.connect_clicked(move |_| {
        if let (Some(p), Some(s)) = (ps.get(c.get()), crate::shell_atual()) {
            crate::image_viewer::open(&s, Some(p.clone()));
        }
    });
    col.append(&acts);
    if debug() {
        eprintln!("debug midia: {} imagens", paths.len());
    }
    col.upcast()
}

/// A playbin for one file (silent in the session test).
fn player(p: &Path) -> Option<gst::Element> {
    if !crate::video_media::init() {
        return None;
    }
    let pb = gst::ElementFactory::make("playbin").property("uri", gtk::gio::File::for_path(p).uri().as_str()).build().ok()?;
    // (a video's picture is not wanted here: the sound only)
    if let Some(fake) = gst::ElementFactory::make("fakesink").build().ok() {
        pb.set_property("video-sink", &fake);
    }
    if std::env::var_os("KALANGO_AREA_TESTE").is_some() || std::env::var("KALANGO_SONS").as_deref() == Ok("nao") {
        pb.set_property("mute", true);
    }
    Some(pb)
}

/// A sound: play/pause, its waveform (where it is in it), the time, and its transcript by the local model.
pub fn audio(p: &Path) -> gtk::Widget {
    let card = caixa(Orientation::Vertical, 8, "midia-audio");
    let top = caixa(Orientation::Horizontal, 12, "");
    let play = gtk::Button::new();
    play.add_css_class("midia-tocar");
    let icon = ui_icon("tocar", "#1C1D24", 14);
    play.set_child(Some(&icon));
    play.set_valign(Align::Center);
    top.append(&play);
    let t = caixa(Orientation::Vertical, 2, "");
    t.set_valign(Align::Center);
    // (a voice message recorded in the composer is that, not its file name; a long name gives way: the card fits a
    // narrow bubble — the side bar's)
    let file = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let nome = if file.starts_with("mensagem-de-voz-") { tr("Voice message").to_string() } else { file };
    let n = rotulo(&nome, "midia-nome");
    n.set_xalign(0.0);
    n.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    n.set_max_width_chars(24);
    t.append(&n);
    let sub = rotulo(tr("sound · transcribed on this computer when you ask"), "midia-sub");
    sub.set_xalign(0.0);
    // (the waveform keeps its room: a long line ends in "…")
    sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
    sub.set_max_width_chars(26);
    t.append(&sub);
    top.append(&t);
    let wave = gtk::DrawingArea::new();
    wave.set_hexpand(true);
    wave.set_content_height(28);
    top.append(&wave);
    let time = rotulo("0:00", "midia-tempo");
    top.append(&time);
    let transcrever = gtk::Button::new();
    transcrever.add_css_class("md-bloco-botao");
    transcrever.set_child(Some(&rotulo(tr("Transcribe"), "")));
    transcrever.set_valign(Align::Center);
    top.append(&transcrever);
    card.append(&top);
    let linhas = caixa(Orientation::Vertical, 4, "midia-transcricao");
    linhas.set_visible(false);
    card.append(&linhas);
    // the samples (16 kHz mono) for the waveform and the length, off the interface thread
    let bars: Rc<RefCell<Vec<f64>>> = Rc::default();
    let pos = Rc::new(Cell::new(0.0f64));
    let len = Rc::new(Cell::new(0.0f64));
    {
        let (b, pp) = (bars.clone(), pos.clone());
        wave.set_draw_func(move |_, cr, w, h| {
            let b = b.borrow();
            if b.is_empty() {
                return;
            }
            let (w, h) = (f64::from(w), f64::from(h));
            let step = w / b.len() as f64;
            let (a, m) = (crate::tema::rgb("#A5A1E2"), crate::tema::rgb("#8E8F99"));
            for (i, v) in b.iter().enumerate() {
                let x = i as f64 * step;
                let bh = (v * h).max(2.0);
                let (r, g, bl) = if x / w <= pp.get() { a } else { m };
                cr.set_source_rgb(r, g, bl);
                cr.rectangle(x + step * 0.2, (h - bh) / 2.0, (step * 0.6).max(1.0), bh);
                let _ = cr.fill();
            }
        });
    }
    let path = p.to_path_buf();
    let (b, l, tl, wv) = (bars.clone(), len.clone(), time.clone(), wave.downgrade());
    crate::lateral::em_segundo_plano(move || crate::video_media::audio(&path).ok(), move |r| {
        let Some(s) = r else { return };
        l.set(s.len() as f64 / 16_000.0);
        tl.set_text(&format!("0:00 / {}", clock(l.get())));
        let n = 72usize;
        let chunk = (s.len() / n).max(1);
        let peaks: Vec<f64> = s.chunks(chunk).take(n).map(|c| c.iter().map(|x| f64::from(x.unsigned_abs())).fold(0.0, f64::max) / 32768.0).collect();
        let top = peaks.iter().cloned().fold(1e-6, f64::max);
        *b.borrow_mut() = peaks.iter().map(|x| (x / top).sqrt()).collect();
        if let Some(w) = wv.upgrade() {
            w.queue_draw();
        }
    });
    // play and pause, the place and the time following the sound
    let pb: Rc<RefCell<Option<gst::Element>>> = Rc::default();
    let tocando = Rc::new(Cell::new(false));
    let (pbc, tc, pth, ic, pp, l, tl, wv) = (pb.clone(), tocando.clone(), p.to_path_buf(), icon.clone(), pos.clone(), len.clone(), time.clone(), wave.downgrade());
    play.connect_clicked(move |_| {
        if pbc.borrow().is_none() {
            *pbc.borrow_mut() = player(&pth);
        }
        let Some(e) = pbc.borrow().clone() else { return };
        let on = !tc.get();
        tc.set(on);
        let _ = e.set_state(if on { gst::State::Playing } else { gst::State::Paused });
        crate::conversa::set_icon(&ic, if on { "pausa" } else { "tocar" }, "#1C1D24");
        if debug() {
            eprintln!("debug midia: áudio {}", if on { "tocando" } else { "pausado" });
        }
        if on {
            let (e, tc, pp, l, tl, wv, ic) = (e.clone(), tc.clone(), pp.clone(), l.clone(), tl.clone(), wv.clone(), ic.clone());
            glib::timeout_add_local(Duration::from_millis(200), move || {
                if !tc.get() {
                    return glib::ControlFlow::Break;
                }
                let at = e.query_position::<gst::ClockTime>().map(|c| c.mseconds() as f64 / 1000.0).unwrap_or(0.0);
                let total = l.get().max(e.query_duration::<gst::ClockTime>().map(|c| c.mseconds() as f64 / 1000.0).unwrap_or(0.0));
                pp.set(if total > 0.0 { at / total } else { 0.0 });
                tl.set_text(&format!("{} / {}", clock(at), clock(total)));
                if let Some(w) = wv.upgrade() {
                    w.queue_draw();
                }
                if total > 0.0 && at >= total - 0.05 {
                    tc.set(false);
                    let _ = e.set_state(gst::State::Ready);
                    crate::conversa::set_icon(&ic, "tocar", "#1C1D24");
                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            });
        }
    });
    // (the sound stops when its card goes: a closed conversation does not keep playing)
    let pbd = pb.clone();
    card.connect_unrealize(move |_| {
        if let Some(e) = pbd.borrow().as_ref() {
            let _ = e.set_state(gst::State::Null);
        }
    });
    crate::central::posicao_debug("midia tocar", play.upcast_ref());
    if debug() {
        eprintln!("debug midia: áudio {}", p.display());
    }
    crate::central::posicao_debug("midia transcrever", transcrever.upcast_ref());
    let path = p.to_path_buf();
    let (lw, sw) = (linhas.downgrade(), sub.downgrade());
    transcrever.connect_clicked(move |b| {
        b.set_sensitive(false);
        if let Some(s) = sw.upgrade() {
            s.set_text(tr("transcribing on the local model…"));
        }
        let (path, lw, sw) = (path.clone(), lw.clone(), sw.clone());
        crate::lateral::em_segundo_plano(move || -> Result<String, String> {
            let samples = crate::video_media::audio(&path)?;
            if crate::video_media::silent(&samples) {
                return Err(tr("there is no speech in it").to_string());
            }
            let wav = crate::voz::private_dir().join(format!("transcrever-{}.wav", std::process::id()));
            std::fs::write(&wav, crate::video_media::wav(&samples)).map_err(|e| e.to_string())?;
            let o = std::process::Command::new(crate::voz::agent_binary()).arg("transcrever").arg(&wav).stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null()).output().map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(&wav);
            let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if o.status.success() && !t.is_empty() { Ok(t) } else { Err(tr("The local model could not do it now.").to_string()) }
        }, move |r| {
            let (Some(l), Some(s)) = (lw.upgrade(), sw.upgrade()) else { return };
            match r {
                Ok(t) => {
                    s.set_text(tr("transcribed on the local model"));
                    for frase in t.split_inclusive(['.', '?', '!']).map(str::trim).filter(|f| !f.is_empty()) {
                        let x = rotulo(frase, "midia-linha");
                        x.set_xalign(0.0);
                        x.set_wrap(true);
                        x.set_selectable(true);
                        l.append(&x);
                    }
                    l.set_visible(true);
                    if debug() {
                        eprintln!("debug midia: transcrito ({} caracteres)", t.chars().count());
                    }
                }
                Err(e) => s.set_text(&e),
            }
        });
    });
    card.upcast()
}

/// A video: its picture at the first second and its length; a click opens it in the Video app.
pub fn video(p: &Path) -> gtk::Widget {
    let card = gtk::Button::new();
    card.add_css_class("midia-video");
    let row = caixa(Orientation::Horizontal, 14, "");
    let pic = gtk::Picture::new();
    pic.set_size_request(192, 108);
    pic.set_content_fit(gtk::ContentFit::Cover);
    pic.add_css_class("midia-video-quadro");
    let over = gtk::Overlay::new();
    over.set_child(Some(&pic));
    let pl = caixa(Orientation::Horizontal, 0, "midia-video-tocar");
    pl.set_halign(Align::Center);
    pl.set_valign(Align::Center);
    pl.append(&ui_icon("tocar", "#E2E2E6", 14));
    over.add_overlay(&pl);
    row.append(&over);
    let t = caixa(Orientation::Vertical, 4, "");
    t.set_valign(Align::Center);
    t.set_hexpand(true);
    let n = rotulo(&p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), "midia-nome");
    n.set_xalign(0.0);
    n.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    n.set_max_width_chars(24);
    t.append(&n);
    let d = rotulo(tr("video"), "midia-sub");
    d.set_xalign(0.0);
    t.append(&d);
    let o = rotulo(tr("Open in Video"), "midia-abrir");
    o.set_xalign(0.0);
    t.append(&o);
    row.append(&t);
    card.set_child(Some(&row));
    let path = p.to_path_buf();
    let (pw, dw) = (pic.downgrade(), d.downgrade());
    crate::lateral::em_segundo_plano(move || crate::video_media::frames(&path, &[1.0], 384, 216).ok().map(|(f, len)| (f.into_iter().next(), len)), move |r| {
        let Some((frame, len)) = r else { return };
        if let (Some(f), Some(pic)) = (frame, pw.upgrade()) {
            let tex = gdk::MemoryTexture::new(f.width as i32, f.height as i32, gdk::MemoryFormat::R8g8b8a8, &glib::Bytes::from_owned(f.data), f.width * 4);
            pic.set_paintable(Some(&tex));
        }
        if let Some(d) = dw.upgrade() {
            d.set_text(&format!("{} · {}", tr("video"), clock(len)));
        }
    });
    let path = p.to_path_buf();
    card.connect_clicked(move |_| {
        if let Some(s) = crate::shell_atual() {
            crate::video_player::open(&s, Some(path.clone()));
        }
    });
    if debug() {
        eprintln!("debug midia: vídeo {}", p.display());
    }
    card.upcast()
}
