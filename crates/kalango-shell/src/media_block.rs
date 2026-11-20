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

/// What is said in a sound, piece by piece: each piece of speech (cut at its pauses) transcribed alone by the local
/// audio model, with the time it starts at. Off the interface's thread.
fn transcribe_pieces(path: &Path) -> Result<Vec<(f64, String)>, String> {
    let samples = crate::video_media::audio(path)?;
    if crate::video_media::silent(&samples) {
        return Err(tr("there is no speech in it").to_string());
    }
    let rate = crate::video_media::RATE as f64;
    let mut out = Vec::new();
    for (a, b) in crate::video_media::speech_chunks(&samples) {
        let wav = crate::voz::private_dir().join(format!("transcrever-{}-{a}.wav", std::process::id()));
        std::fs::write(&wav, crate::video_media::wav(&samples[a..b])).map_err(|e| e.to_string())?;
        let o = std::process::Command::new(crate::voz::agent_binary()).arg("transcrever").arg(&wav).stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()).output().map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&wav);
        let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if !o.status.success() {
            return Err(tr("The local model could not do it now.").to_string());
        }
        if !t.is_empty() {
            out.push((a as f64 / rate, t));
        }
    }
    if out.is_empty() {
        return Err(tr("there is no speech in it").to_string());
    }
    Ok(out)
}

/// The piece being heard at `at` seconds: the last one that started.
pub fn current_piece(starts: &[f64], at: f64) -> Option<usize> {
    starts.iter().rposition(|s| *s <= at + 0.05)
}

/// Goes to `t` seconds at `rate` (the speed button keeps the place).
fn seek(e: &gst::Element, t: f64, rate: f64) {
    let at = Some(gst::ClockTime::from_nseconds((t.max(0.0) * 1e9) as u64));
    let _ = e.seek(rate, gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE, gst::SeekType::Set, at, gst::SeekType::None, gst::ClockTime::NONE);
}

/// A sound (design "Chat · mídia"): play/pause, its waveform (where it is in it), the time, the speed and a copy in
/// Downloads; "Transcrever" writes what is said under it, line by line with the time each line starts — the line
/// being heard lights up as it plays, and a click on a line goes there.
pub fn audio(p: &Path) -> gtk::Widget {
    let card = caixa(Orientation::Vertical, 0, "midia-audio");
    let top = caixa(Orientation::Horizontal, 12, "midia-audio-topo");
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
    let speed = gtk::Button::new();
    speed.add_css_class("midia-velocidade");
    let speed_l = rotulo("1×", "");
    speed.set_child(Some(&speed_l));
    speed.set_valign(Align::Center);
    speed.set_tooltip_text(Some(tr("Speed")));
    top.append(&speed);
    let save = gtk::Button::new();
    save.add_css_class("midia-baixar");
    save.set_child(Some(&ui_icon("baixar", "#8E8F99", 16)));
    save.set_valign(Align::Center);
    save.set_tooltip_text(Some(tr("Save in Downloads")));
    top.append(&save);
    let transcrever = gtk::Button::new();
    transcrever.add_css_class("md-bloco-botao");
    transcrever.add_css_class("midia-transcrever");
    transcrever.set_child(Some(&rotulo(tr("Transcribe"), "")));
    transcrever.set_valign(Align::Center);
    top.append(&transcrever);
    card.append(&top);
    let linhas = caixa(Orientation::Vertical, 2, "midia-transcricao");
    linhas.set_visible(false);
    card.append(&linhas);
    // the transcript's lines: when each starts, and its row
    let pieces: Rc<RefCell<Vec<(f64, gtk::Widget)>>> = Rc::default();
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
    // play and pause, the place, the time and the line being heard following the sound
    let pb: Rc<RefCell<Option<gst::Element>>> = Rc::default();
    let tocando = Rc::new(Cell::new(false));
    let rate = Rc::new(Cell::new(1.0f64));
    let follow: Rc<dyn Fn()> = {
        let (pbc, tc, pp, l, tl, wv, ic, pc) = (pb.clone(), tocando.clone(), pos.clone(), len.clone(), time.clone(), wave.downgrade(), icon.clone(), pieces.clone());
        Rc::new(move || {
            let (pbc, tc, pp, l, tl, wv, ic, pc) = (pbc.clone(), tc.clone(), pp.clone(), l.clone(), tl.clone(), wv.clone(), ic.clone(), pc.clone());
            let lit: Rc<Cell<Option<usize>>> = Rc::default();
            glib::timeout_add_local(Duration::from_millis(200), move || {
                if !tc.get() {
                    return glib::ControlFlow::Break;
                }
                let Some(e) = pbc.borrow().clone() else { return glib::ControlFlow::Break };
                let at = e.query_position::<gst::ClockTime>().map(|c| c.mseconds() as f64 / 1000.0).unwrap_or(0.0);
                let total = l.get().max(e.query_duration::<gst::ClockTime>().map(|c| c.mseconds() as f64 / 1000.0).unwrap_or(0.0));
                pp.set(if total > 0.0 { at / total } else { 0.0 });
                tl.set_text(&format!("{} / {}", clock(at), clock(total)));
                if let Some(w) = wv.upgrade() {
                    w.queue_draw();
                }
                let pcs = pc.borrow();
                let now = current_piece(&pcs.iter().map(|(s, _)| *s).collect::<Vec<_>>(), at);
                if now != lit.get() {
                    if let Some(old) = lit.get().and_then(|i| pcs.get(i)) {
                        old.1.remove_css_class("atual");
                    }
                    if let Some(new) = now.and_then(|i| pcs.get(i)) {
                        new.1.add_css_class("atual");
                        if debug() {
                            eprintln!("debug midia: linha {} acesa ({})", now.unwrap_or(0) + 1, clock(new.0));
                        }
                    }
                    lit.set(now);
                }
                if total > 0.0 && at >= total - 0.05 {
                    tc.set(false);
                    let _ = e.set_state(gst::State::Ready);
                    crate::conversa::set_icon(&ic, "tocar", "#1C1D24");
                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            });
        })
    };
    let start_or_toggle: Rc<dyn Fn(Option<f64>)> = {
        let (pbc, tc, pth, ic, rt, fl) = (pb.clone(), tocando.clone(), p.to_path_buf(), icon.clone(), rate.clone(), follow.clone());
        Rc::new(move |at: Option<f64>| {
            if pbc.borrow().is_none() {
                *pbc.borrow_mut() = player(&pth);
            }
            let Some(e) = pbc.borrow().clone() else { return };
            // a line clicked: play from there; the button: play or pause
            let on = at.is_some() || !tc.get();
            let was = tc.replace(on);
            if on {
                let _ = e.set_state(gst::State::Playing);
                if let Some(t) = at {
                    // (the pipeline takes a moment to be ready for a seek the first time)
                    let _ = e.state(gst::ClockTime::from_seconds(2));
                    seek(&e, t, rt.get());
                }
            } else {
                let _ = e.set_state(gst::State::Paused);
            }
            crate::conversa::set_icon(&ic, if on { "pausa" } else { "tocar" }, "#1C1D24");
            if debug() {
                eprintln!("debug midia: áudio {}", if on { "tocando" } else { "pausado" });
            }
            if on && !was {
                fl();
            }
        })
    };
    let st = start_or_toggle.clone();
    play.connect_clicked(move |_| st(None));
    // the speed: 1×, 1,5×, 2×
    let (pbc, rt, sl) = (pb.clone(), rate.clone(), speed_l.clone());
    speed.connect_clicked(move |_| {
        let next = match rt.get() {
            r if r < 1.25 => 1.5,
            r if r < 1.75 => 2.0,
            _ => 1.0,
        };
        rt.set(next);
        sl.set_text(&format!("{}×", format!("{next}").replace('.', ",")));
        if let Some(e) = pbc.borrow().as_ref() {
            let at = e.query_position::<gst::ClockTime>().map(|c| c.mseconds() as f64 / 1000.0).unwrap_or(0.0);
            seek(e, at, next);
        }
        if debug() {
            eprintln!("debug midia: velocidade {next}");
        }
    });
    // a copy in Downloads
    let (pth, sw) = (p.to_path_buf(), sub.downgrade());
    save.connect_clicked(move |b| {
        let dir = crate::pastas::downloads();
        let name = pth.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        let mut to = dir.join(&name);
        let mut i = 2;
        while to.exists() && to != pth {
            let stem = pth.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = pth.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
            to = dir.join(format!("{stem} ({i}){ext}"));
            i += 1;
        }
        let ok = to == pth || std::fs::create_dir_all(&dir).and_then(|_| std::fs::copy(&pth, &to)).is_ok();
        if let Some(s) = sw.upgrade() {
            s.set_text(if ok { tr("saved in Downloads") } else { tr("could not save it in Downloads") });
        }
        b.set_sensitive(!ok);
        if debug() {
            eprintln!("debug midia: salvo em {} ({ok})", to.display());
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
    let (lw, sw, pc, st) = (linhas.downgrade(), sub.downgrade(), pieces.clone(), start_or_toggle.clone());
    transcrever.connect_clicked(move |b| {
        b.set_sensitive(false);
        if let Some(s) = sw.upgrade() {
            s.set_text(tr("transcribing on the local model…"));
        }
        let (path, lw, sw, pc, st, bw) = (path.clone(), lw.clone(), sw.clone(), pc.clone(), st.clone(), b.downgrade());
        crate::lateral::em_segundo_plano(move || transcribe_pieces(&path), move |r| {
            let (Some(l), Some(s)) = (lw.upgrade(), sw.upgrade()) else { return };
            match r {
                Ok(lines) => {
                    s.set_text(tr("transcribed on the local model"));
                    if let Some(b) = bw.upgrade() {
                        b.set_visible(false);
                    }
                    for (start, text) in &lines {
                        let row = gtk::Button::new();
                        row.add_css_class("midia-linha-botao");
                        let r = caixa(Orientation::Horizontal, 10, "");
                        let tm = rotulo(&clock(*start), "midia-linha-tempo");
                        tm.set_valign(Align::Start);
                        r.append(&tm);
                        let x = rotulo(text, "midia-linha");
                        x.set_xalign(0.0);
                        x.set_wrap(true);
                        x.set_hexpand(true);
                        r.append(&x);
                        row.set_child(Some(&r));
                        let (st, at) = (st.clone(), *start);
                        row.connect_clicked(move |_| st(Some(at)));
                        l.append(&row);
                        pc.borrow_mut().push((*start, row.upcast()));
                    }
                    l.set_visible(true);
                    if let Some(first) = pc.borrow().first() {
                        crate::central::posicao_debug("midia linha 1", &first.1);
                    }
                    if debug() {
                        eprintln!("debug midia: transcrito ({} linhas: {})", lines.len(),
                            lines.iter().map(|(t, x)| format!("{} «{}»", clock(*t), x.chars().take(40).collect::<String>())).collect::<Vec<_>>().join("; "));
                    }
                }
                Err(e) => {
                    s.set_text(&e);
                    if let Some(b) = bw.upgrade() {
                        b.set_sensitive(true);
                    }
                }
            }
        });
    });
    card.upcast()
}

/// "0:00" for an hour or less, else "1:02:03"; the chapters' times as the design writes them ("07:15").
fn mmss(s: f64) -> String {
    let s = s.max(0.0) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{:02}:{:02}", s / 60, s % 60) }
}

/// A video (design "Chat · mídia"): its picture with the length (a click opens it in the Video app), its name, length
/// and size, and its chapters — the scenes the local vision model found and named (the Video app's, kept; found here
/// when the video is new). A chapter chosen: "Ir para" opens the video there, "Cortar este trecho" saves that piece
/// as a new video in Vídeos.
pub fn video(p: &Path) -> gtk::Widget {
    let card = caixa(Orientation::Horizontal, 14, "midia-video");
    let pic = gtk::Picture::new();
    pic.set_size_request(300, 169);
    pic.set_content_fit(gtk::ContentFit::Cover);
    pic.add_css_class("midia-video-quadro");
    let over = gtk::Overlay::new();
    over.set_child(Some(&pic));
    let pl = caixa(Orientation::Horizontal, 0, "midia-video-tocar");
    pl.set_halign(Align::Center);
    pl.set_valign(Align::Center);
    pl.append(&ui_icon("tocar", "#E2E2E6", 14));
    over.add_overlay(&pl);
    let dur = rotulo("", "midia-video-duracao");
    dur.set_halign(Align::End);
    dur.set_valign(Align::End);
    dur.set_margin_end(8);
    dur.set_margin_bottom(8);
    dur.set_visible(false);
    over.add_overlay(&dur);
    let open = gtk::Button::new();
    open.add_css_class("midia-video-abrir");
    open.set_child(Some(&over));
    open.set_valign(Align::Start);
    open.set_tooltip_text(Some(tr("Open in Video")));
    card.append(&open);
    let t = caixa(Orientation::Vertical, 4, "");
    t.set_hexpand(true);
    let n = rotulo(&p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), "midia-nome");
    n.set_xalign(0.0);
    n.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    n.set_max_width_chars(24);
    t.append(&n);
    let d = rotulo(tr("video"), "midia-sub");
    d.set_xalign(0.0);
    d.set_ellipsize(gtk::pango::EllipsizeMode::End);
    t.append(&d);
    let lista = caixa(Orientation::Vertical, 0, "midia-capitulos");
    t.append(&lista);
    let acts = caixa(Orientation::Horizontal, 8, "midia-video-acoes");
    acts.set_visible(false);
    let ir = gtk::Button::new();
    ir.add_css_class("midia-video-ir");
    let ir_r = caixa(Orientation::Horizontal, 6, "");
    ir_r.append(&ui_icon("tocar", "#A5A1E2", 12));
    let ir_l = rotulo("", "");
    ir_r.append(&ir_l);
    ir.set_child(Some(&ir_r));
    acts.append(&ir);
    let cortar = gtk::Button::new();
    cortar.add_css_class("midia-video-cortar");
    let c_r = caixa(Orientation::Horizontal, 6, "");
    c_r.append(&ui_icon("recortar", "#E2E2E6", 14));
    c_r.append(&rotulo(tr("Cut this part"), ""));
    cortar.set_child(Some(&c_r));
    acts.append(&cortar);
    t.append(&acts);
    card.append(&t);
    let length = Rc::new(Cell::new(0.0f64));
    let chosen: Rc<Cell<Option<(f64, f64)>>> = Rc::default();
    // the chapters on the card: a click chooses one
    let show: Rc<dyn Fn(Vec<(f64, String)>)> = {
        let (lw, aw, il, ch, ln, dw) = (lista.downgrade(), acts.downgrade(), ir_l.clone(), chosen.clone(), length.clone(), d.downgrade());
        Rc::new(move |chs: Vec<(f64, String)>| {
            let (Some(l), Some(a)) = (lw.upgrade(), aw.upgrade()) else { return };
            while let Some(c) = l.first_child() {
                l.remove(&c);
            }
            if chs.len() < 2 {
                return;
            }
            if let Some(d) = dw.upgrade() {
                d.set_text(&format!("{} · {}", d.text(), tr("chapters made on the local model")));
            }
            let rows: Rc<RefCell<Vec<gtk::Button>>> = Rc::default();
            for (i, (s, name)) in chs.iter().enumerate() {
                let end = chs.get(i + 1).map(|c| c.0).unwrap_or_else(|| ln.get());
                let b = gtk::Button::new();
                b.add_css_class("midia-capitulo");
                let r = caixa(Orientation::Horizontal, 10, "");
                r.append(&rotulo(&mmss(*s), "midia-capitulo-tempo"));
                let x = rotulo(if name.is_empty() { tr("Scene") } else { name }, "midia-capitulo-nome");
                x.set_xalign(0.0);
                x.set_ellipsize(gtk::pango::EllipsizeMode::End);
                r.append(&x);
                b.set_child(Some(&r));
                let (rs, il, ch, aw2, s) = (rows.clone(), il.clone(), ch.clone(), a.downgrade(), *s);
                b.connect_clicked(move |me| {
                    for o in rs.borrow().iter() {
                        o.remove_css_class("ativo");
                    }
                    me.add_css_class("ativo");
                    ch.set(Some((s, end)));
                    il.set_text(&trf("Go to {time}", &[("time", mmss(s))]));
                    if let Some(a) = aw2.upgrade() {
                        a.set_visible(true);
                    }
                    if debug() {
                        eprintln!("debug midia: capítulo {} escolhido", mmss(s));
                    }
                });
                crate::central::posicao_debug(&format!("midia capitulo {}", i + 1), b.upcast_ref());
                l.append(&b);
                rows.borrow_mut().push(b);
            }
            // the second chapter chosen at first (the first starts with the video: "Ir para 00:00" says little)
            if let Some(b) = rows.borrow().get(1) {
                b.emit_clicked();
            }
            if debug() {
                eprintln!("debug midia: capítulos no cartão ({}): {}", chs.len(), chs.iter().map(|(t, n)| format!("{} «{n}»", mmss(*t))).collect::<Vec<_>>().join("; "));
            }
        })
    };
    let (path, path2) = (p.to_path_buf(), p.to_path_buf());
    let (pw, dw, durw, ln, sh) = (pic.downgrade(), d.downgrade(), dur.downgrade(), length.clone(), show.clone());
    crate::lateral::em_segundo_plano(move || {
        let r = crate::video_media::frames(&path, &[1.0], 384, 216).ok().map(|(f, len)| (f.into_iter().next(), len));
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        (r, size)
    }, move |(r, size)| {
        let Some((frame, len)) = r else { return };
        ln.set(len);
        if let Some(f) = &frame {
            if let Some(pic) = pw.upgrade() {
                let tex = gdk::MemoryTexture::new(f.width as i32, f.height as i32, gdk::MemoryFormat::R8g8b8a8, &glib::Bytes::from(f.data.as_slice()), f.width * 4);
                pic.set_paintable(Some(&tex));
            }
        }
        if let Some(d) = durw.upgrade() {
            d.set_text(&mmss(len));
            d.set_visible(true);
        }
        if let Some(d) = dw.upgrade() {
            d.set_text(&format!("{} · {}", clock(len), glib::format_size(size)));
        }
        let path = path2;
        let cached = crate::video_player::cached_chapters(&path);
        if !cached.is_empty() {
            sh(cached);
            return;
        }
        // new to the system: the agent finds the chapters (kept for the Video app too)
        let sh = sh.clone();
        let bin = crate::voz::agent_binary();
        crate::lateral::em_segundo_plano(move || {
            let r = crate::video_player::find_chapters_now(&path, &bin);
            if let Ok(ch) = &r {
                crate::video_player::save_chapters(&path, ch);
            }
            r
        }, move |r| match r {
            Ok(ch) => sh(ch),
            Err(e) => eprintln!("kalango-shell: mídia: capítulos: {e}"),
        });
    });
    let path = p.to_path_buf();
    open.connect_clicked(move |_| {
        if let Some(s) = crate::shell_atual() {
            crate::video_player::open(&s, Some(path.clone()));
        }
    });
    let (path, ch) = (p.to_path_buf(), chosen.clone());
    ir.connect_clicked(move |_| {
        let (Some(s), Some((at, _))) = (crate::shell_atual(), ch.get()) else { return };
        if debug() {
            eprintln!("debug midia: ir para {}", mmss(at));
        }
        crate::video_player::open_at(&s, path.clone(), at);
    });
    let (path, ch, dw) = (p.to_path_buf(), chosen.clone(), d.downgrade());
    cortar.connect_clicked(move |b| {
        let Some((a, z)) = ch.get() else { return };
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let dir = crate::pastas::especial(glib::UserDirectory::Videos, "Vídeos");
        let out = dir.join(format!("{stem} — {}.webm", mmss(a).replace(':', "m") + "s"));
        b.set_sensitive(false);
        if let Some(d) = dw.upgrade() {
            d.set_text(&trf("cutting {from}–{to}…", &[("from", mmss(a)), ("to", mmss(z))]));
        }
        let (path, bw, dw) = (path.clone(), b.downgrade(), dw.clone());
        crate::lateral::em_segundo_plano(move || {
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            crate::video_media::cut(&path, a, z, &out).map(|_| out)
        }, move |r: Result<PathBuf, String>| {
            if let Some(b) = bw.upgrade() {
                b.set_sensitive(true);
            }
            let Some(d) = dw.upgrade() else { return };
            match r {
                Ok(o) => {
                    d.set_text(&trf("saved in Vídeos as “{file}”", &[("file", o.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())]));
                    if debug() {
                        eprintln!("debug midia: trecho cortado {} ({} bytes)", o.display(), std::fs::metadata(&o).map(|m| m.len()).unwrap_or(0));
                    }
                }
                Err(e) => {
                    d.set_text(&trf("could not cut it: {reason}", &[("reason", e)]));
                    eprintln!("kalango-shell: mídia: cortar: {}", d.text());
                }
            }
        });
    });
    crate::central::posicao_debug("midia ir", ir.upcast_ref());
    crate::central::posicao_debug("midia cortar", cortar.upcast_ref());
    if debug() {
        eprintln!("debug midia: vídeo {}", p.display());
    }
    card.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_being_heard_and_the_chapters_times() {
        let starts = [0.0, 31.0, 38.0];
        assert_eq!(current_piece(&starts, 0.0), Some(0));
        assert_eq!(current_piece(&starts, 35.2), Some(1));
        assert_eq!(current_piece(&starts, 120.0), Some(2));
        assert_eq!(current_piece(&[], 3.0), None);
        assert_eq!(mmss(435.0), "07:15");
        assert_eq!(mmss(3725.0), "1:02:05");
    }
}
