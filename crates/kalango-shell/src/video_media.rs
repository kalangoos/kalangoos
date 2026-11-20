//! What the Video app reads from a file with GStreamer, aside from playing it: frames at given times (for the
//! chapters the agent finds and names), the sound as 16 kHz mono (for the live captions and the summary), and the
//! scene changes between frames. Everything here runs in a background thread.

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::path::{Path, PathBuf};

/// The sample rate the audio model listens at.
pub const RATE: usize = 16_000;

pub fn init() -> bool {
    gst::init().is_ok()
}

fn uri(p: &Path) -> String {
    gtk::gio::prelude::FileExt::uri(&gtk::gio::File::for_path(p)).to_string()
}

/// A frame: its time (s) and its RGBA pixels.
pub struct Frame {
    pub t: f64,
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

/// Frames at `times` (seconds), `width` × `height` RGBA each (letterboxed when the video is not of that shape), and
/// the video's length. Accurate seeks: a frame is the one at that time, not the keyframe before it.
pub fn frames(path: &Path, times: &[f64], width: usize, height: usize) -> Result<(Vec<Frame>, f64), String> {
    if !init() {
        return Err("GStreamer".into());
    }
    let desc = format!(
        "uridecodebin uri=\"{}\" caps=video/x-raw ! videoconvert ! videoscale add-borders=true ! \
         video/x-raw,format=RGBA,width={width},height={height},pixel-aspect-ratio=1/1 ! appsink name=s sync=false max-buffers=1",
        uri(path).replace('"', "%22")
    );
    let pipe = gst::parse::launch(&desc).map_err(|e| e.to_string())?.downcast::<gst::Pipeline>().map_err(|_| "pipeline")?;
    let sink = pipe.by_name("s").and_downcast::<gst_app::AppSink>().ok_or("appsink")?;
    let done = |pipe: &gst::Pipeline| {
        let _ = pipe.set_state(gst::State::Null);
    };
    pipe.set_state(gst::State::Paused).map_err(|e| e.to_string())?;
    let (r, _, _) = pipe.state(gst::ClockTime::from_seconds(15));
    if r.is_err() {
        done(&pipe);
        return Err(error_of(&pipe).unwrap_or_else(|| "the video could not be read".into()));
    }
    let length = pipe.query_duration::<gst::ClockTime>().map(|d| d.nseconds() as f64 / 1e9).unwrap_or(0.0);
    let mut out = Vec::new();
    for &t in times {
        let t = if length > 0.0 { t.min(length - 0.05).max(0.0) } else { t };
        if pipe.seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE, gst::ClockTime::from_nseconds((t * 1e9) as u64)).is_err() {
            continue;
        }
        let _ = pipe.state(gst::ClockTime::from_seconds(10));
        let Ok(sample) = sink.pull_preroll() else { continue };
        let Some(buf) = sample.buffer() else { continue };
        let Ok(map) = buf.map_readable() else { continue };
        let stride = sample.caps().and_then(|c| gstreamer_video::VideoInfo::from_caps(c).ok()).map(|i| i.stride()[0] as usize).unwrap_or(width * 4);
        let mut data = Vec::with_capacity(width * height * 4);
        for row in 0..height {
            let a = row * stride;
            if let Some(r) = map.as_slice().get(a..a + width * 4) {
                data.extend_from_slice(r);
            }
        }
        if data.len() == width * height * 4 {
            out.push(Frame { t, width, height, data });
        }
    }
    done(&pipe);
    Ok((out, length))
}

fn error_of(pipe: &gst::Pipeline) -> Option<String> {
    let bus = pipe.bus()?;
    while let Some(m) = bus.pop() {
        if let gst::MessageView::Error(e) = m.view() {
            return Some(e.error().to_string());
        }
    }
    None
}

/// The whole sound of a file as 16 kHz mono samples (empty when it has none).
pub fn audio(path: &Path) -> Result<Vec<i16>, String> {
    if !init() {
        return Err("GStreamer".into());
    }
    let desc = format!(
        "uridecodebin uri=\"{}\" caps=audio/x-raw ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,rate={RATE},channels=1,layout=interleaved ! \
         appsink name=s sync=false",
        uri(path).replace('"', "%22")
    );
    let pipe = gst::parse::launch(&desc).map_err(|e| e.to_string())?.downcast::<gst::Pipeline>().map_err(|_| "pipeline")?;
    let sink = pipe.by_name("s").and_downcast::<gst_app::AppSink>().ok_or("appsink")?;
    pipe.set_state(gst::State::Playing).map_err(|e| e.to_string())?;
    let mut out: Vec<i16> = Vec::new();
    // (a file with no sound: the pipeline errors out or never prerolls; give up after a while with nothing)
    let mut idle = 0;
    loop {
        match sink.try_pull_sample(gst::ClockTime::from_mseconds(500)) {
            Some(s) => {
                idle = 0;
                if let Some(b) = s.buffer() {
                    if let Ok(m) = b.map_readable() {
                        out.extend(m.as_slice().chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])));
                    }
                }
            }
            None if sink.is_eos() => break,
            None => {
                if error_of(&pipe).is_some() {
                    break;
                }
                idle += 1;
                if idle > 20 {
                    break;
                }
            }
        }
    }
    let _ = pipe.set_state(gst::State::Null);
    Ok(out)
}

/// Where speech is in a sound (16 kHz mono): pieces of 2 to 20 s cut at its pauses (300 ms or more under the
/// threshold), as (start, end) in samples — each one is transcribed alone, so its words get the time it starts at.
/// The pieces with no voice at all are left out.
pub fn speech_chunks(samples: &[i16]) -> Vec<(usize, usize)> {
    const FRAME: usize = RATE / 50; // 20 ms
    let rms: Vec<f64> = samples.chunks(FRAME).map(|c| (c.iter().map(|x| f64::from(*x) * f64::from(*x)).sum::<f64>() / c.len().max(1) as f64).sqrt()).collect();
    if rms.is_empty() {
        return vec![];
    }
    // the floor: the quietest tenth of the sound
    let mut sorted = rms.clone();
    sorted.sort_by(f64::total_cmp);
    let floor = sorted[sorted.len() / 10];
    let threshold = (floor * 3.0).max(300.0);
    let (min_len, max_len, pause) = (2 * 50, 20 * 50, 15);
    let mut out = Vec::new();
    let (mut start, mut quiet, mut voiced) = (0usize, 0usize, false);
    for (i, r) in rms.iter().enumerate() {
        if *r < threshold {
            quiet += 1;
        } else {
            quiet = 0;
            voiced = true;
        }
        let len = i + 1 - start;
        if (quiet >= pause && len >= min_len) || len >= max_len {
            if voiced {
                out.push((start * FRAME, ((i + 1) * FRAME).min(samples.len())));
            }
            start = i + 1;
            voiced = false;
            quiet = 0;
        }
    }
    if voiced && start < rms.len() {
        match out.last_mut() {
            // (a short end goes with the piece before it)
            Some(last) if rms.len() - start < min_len && last.1 == start * FRAME => last.1 = samples.len(),
            _ => out.push((start * FRAME, samples.len())),
        }
    }
    out
}

/// A piece of a video (`start` to `end`, in seconds) as a new WebM file — VP8 and Vorbis, which the system's own
/// GStreamer writes ("Cortar este trecho" in the conversation). The file is decoded from its start and what is
/// outside the piece dropped (a muxer that has begun cannot take a seek), the piece's times moved to start at 0.
pub fn cut(path: &Path, start: f64, end: f64, out: &Path) -> Result<(), String> {
    if !init() {
        return Err("GStreamer".into());
    }
    let pipe = gst::Pipeline::new();
    let src = gst::ElementFactory::make("uridecodebin").property("uri", uri(path)).build().map_err(|e| e.to_string())?;
    let mux = gst::ElementFactory::make("webmmux").build().map_err(|e| e.to_string())?;
    let sink = gst::ElementFactory::make("filesink").property("location", out.to_string_lossy().to_string()).build().map_err(|e| e.to_string())?;
    pipe.add_many([&src, &mux, &sink]).map_err(|e| e.to_string())?;
    mux.link(&sink).map_err(|e| e.to_string())?;
    let (from, to) = ((start.max(0.0) * 1e9) as u64, (end.max(start) * 1e9) as u64);
    let (pw, mw) = (pipe.downgrade(), mux.downgrade());
    src.connect_pad_added(move |_, pad| {
        let (Some(pipe), Some(mux)) = (pw.upgrade(), mw.upgrade()) else { return };
        let caps = pad.current_caps().unwrap_or_else(|| pad.query_caps(None));
        let name = caps.structure(0).map(|s| s.name().to_string()).unwrap_or_default();
        let chain: &[&str] = if name.starts_with("video/") {
            &["queue", "videoconvert", "vp8enc"]
        } else if name.starts_with("audio/") {
            &["queue", "audioconvert", "audioresample", "vorbisenc"]
        } else {
            return;
        };
        let els: Vec<gst::Element> = chain.iter().filter_map(|n| gst::ElementFactory::make(n).build().ok()).collect();
        if els.len() != chain.len() || pipe.add_many(&els).is_err() || gst::Element::link_many(&els).is_err() {
            return;
        }
        if let Some(e) = els.iter().find(|e| e.factory().is_some_and(|f| f.name() == "vp8enc")) {
            // (as fast as it can, good enough for a clip)
            e.set_property("deadline", 1i64);
        }
        let last = els.last().unwrap();
        if last.link(&mux).is_err() {
            return;
        }
        for e in &els {
            let _ = e.sync_state_with_parent();
        }
        let Some(sp) = els[0].static_pad("sink") else { return };
        // only the piece: before it dropped, after it the end of this stream; its times from 0
        let ended = std::sync::atomic::AtomicBool::new(false);
        let peer = sp.clone();
        pad.add_probe(gst::PadProbeType::BUFFER, move |_, info| {
            let Some(pts) = info.buffer().and_then(|b| b.pts()).map(|t| t.nseconds()) else { return gst::PadProbeReturn::Ok };
            if pts < from {
                return gst::PadProbeReturn::Drop;
            }
            if pts >= to {
                if !ended.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    peer.send_event(gst::event::Eos::new());
                }
                return gst::PadProbeReturn::Drop;
            }
            gst::PadProbeReturn::Ok
        });
        pad.set_offset(-(from as i64));
        let _ = pad.link(&sp);
    });
    let bus = pipe.bus().ok_or("bus")?;
    pipe.set_state(gst::State::Playing).map_err(|e| e.to_string())?;
    let mut result = Err("tempo esgotado".to_string());
    for msg in bus.iter_timed(gst::ClockTime::from_seconds(900)) {
        match msg.view() {
            gst::MessageView::Eos(_) => {
                result = Ok(());
                break;
            }
            gst::MessageView::Error(e) => {
                result = Err(e.error().to_string());
                break;
            }
            _ => {}
        }
    }
    let _ = pipe.set_state(gst::State::Null);
    if result.is_err() {
        let _ = std::fs::remove_file(out);
    }
    result
}

/// A WAV file (16-bit mono) of samples.
pub fn wav(samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + samples.len() * 2);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&(RATE as u32).to_le_bytes());
    v.extend_from_slice(&(RATE as u32 * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

/// Whether a stretch of sound is all but silent (nothing to transcribe).
pub fn silent(samples: &[i16]) -> bool {
    if samples.is_empty() {
        return true;
    }
    let rms = (samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum::<f64>() / samples.len() as f64).sqrt();
    rms < 200.0
}

/// A frame's look in a few numbers: the mean colour of each cell of a 4 × 4 grid.
fn look(f: &Frame) -> Vec<f64> {
    let mut v = vec![0.0; 48];
    let mut n = [0usize; 16];
    for y in 0..f.height {
        for x in 0..f.width {
            let cell = (y * 4 / f.height) * 4 + x * 4 / f.width;
            let i = (y * f.width + x) * 4;
            for c in 0..3 {
                v[cell * 3 + c] += f64::from(f.data[i + c]);
            }
            n[cell] += 1;
        }
    }
    for (cell, k) in n.iter().enumerate() {
        for c in 0..3 {
            v[cell * 3 + c] /= (*k).max(1) as f64;
        }
    }
    v
}

/// Where the scenes change: the times (s) after which the picture is another, the strongest first kept, at least
/// `min_gap` apart, at most `max` of them, in order. The start (0) is not among them.
pub fn scene_changes(frames: &[Frame], min_gap: f64, max: usize) -> Vec<f64> {
    let looks: Vec<Vec<f64>> = frames.iter().map(look).collect();
    let mut cand: Vec<(f64, f64)> = Vec::new();
    for i in 1..frames.len() {
        let d = looks[i].iter().zip(&looks[i - 1]).map(|(a, b)| (a - b).abs()).sum::<f64>() / 48.0 / 255.0;
        if d > 0.12 {
            cand.push((d, frames[i].t));
        }
    }
    cand.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut kept: Vec<f64> = Vec::new();
    for (_, t) in cand {
        if kept.len() >= max {
            break;
        }
        if t >= min_gap && kept.iter().all(|k| (k - t).abs() >= min_gap) {
            kept.push(t);
        }
    }
    kept.sort_by(f64::total_cmp);
    kept
}

/// The exact time a scene changes, between `before` (still the old scene) and `after` (the new one): halved four
/// times (to an eighth of the step).
pub fn refine_change(path: &Path, before: f64, after: f64) -> f64 {
    let Ok((f, _)) = frames(path, &[before], 64, 36) else { return after };
    let Some(old) = f.first().map(look) else { return after };
    let (mut lo, mut hi) = (before, after);
    for _ in 0..4 {
        let mid = (lo + hi) / 2.0;
        let Ok((m, _)) = frames(path, &[mid], 64, 36) else { break };
        let Some(m) = m.first() else { break };
        let d = look(m).iter().zip(&old).map(|(a, b)| (a - b).abs()).sum::<f64>() / 48.0 / 255.0;
        if d > 0.12 {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

/// A PNG of a frame (for the vision model).
pub fn png(f: &Frame, path: &Path) -> Result<(), String> {
    let pb = gtk::gdk_pixbuf::Pixbuf::from_bytes(&gtk::glib::Bytes::from(f.data.as_slice()), gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, f.width as i32, f.height as i32, (f.width * 4) as i32);
    pb.savev(path, "png", &[]).map_err(|e| e.to_string())
}

/// Where the app keeps what it learnt of a video (its chapters, what is said in it), by its path and date.
pub fn cache_file(path: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    std::fs::metadata(path).and_then(|m| m.modified()).ok().hash(&mut h);
    gtk::glib::user_cache_dir().join("kalango").join("video").join(format!("{:016x}.json", h.finish()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(t: f64, rgb: [u8; 3]) -> Frame {
        Frame { t, width: 8, height: 8, data: (0..64).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect() }
    }

    #[test]
    fn a_piece_of_a_video_becomes_a_shorter_video() {
        // (only where GStreamer has the encoders: the system's plugins-good)
        if !init() || ["vp8enc", "webmmux", "vorbisenc", "videotestsrc"].iter().any(|e| gst::ElementFactory::find(e).is_none()) {
            return;
        }
        let dir = std::env::temp_dir().join(format!("kalango-cortar-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("fonte.webm");
        let desc = format!("webmmux name=m ! filesink location={} videotestsrc num-buffers=300 ! video/x-raw,width=160,height=90,framerate=30/1 ! vp8enc deadline=1 ! m. \
            audiotestsrc num-buffers=430 ! audioconvert ! vorbisenc ! m.", src.display());
        let p = gst::parse::launch(&desc).unwrap();
        p.set_state(gst::State::Playing).unwrap();
        let bus = p.bus().unwrap();
        let _ = bus.timed_pop_filtered(gst::ClockTime::from_seconds(60), &[gst::MessageType::Eos, gst::MessageType::Error]);
        let _ = p.set_state(gst::State::Null);
        let out = dir.join("trecho.webm");
        cut(&src, 3.0, 6.0, &out).unwrap();
        let (_, len) = frames(&out, &[], 8, 8).unwrap();
        assert!((2.5..3.6).contains(&len), "{len}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn speech_is_cut_at_its_pauses() {
        // 3 s of voice, 0.5 s of silence, 4 s of voice, 2 s of silence
        let tone = |secs: f64| (0..(secs * RATE as f64) as usize).map(|i| ((i as f64 * 0.05).sin() * 8000.0) as i16).collect::<Vec<_>>();
        let quiet = |secs: f64| vec![0i16; (secs * RATE as f64) as usize];
        let s: Vec<i16> = [tone(3.0), quiet(0.5), tone(4.0), quiet(2.0)].concat();
        let c = speech_chunks(&s);
        assert_eq!(c.len(), 2, "{c:?}");
        let secs = |x: usize| x as f64 / RATE as f64;
        assert!(secs(c[0].0) < 0.01 && (3.2..3.6).contains(&secs(c[0].1)), "{c:?}");
        assert!((3.2..3.6).contains(&secs(c[1].0)), "{c:?}");
        // silence alone: nothing to transcribe
        assert!(speech_chunks(&quiet(5.0)).is_empty());
    }

    #[test]
    fn finds_where_the_picture_changes() {
        let mut f = Vec::new();
        for t in 0..30 {
            let c = if t < 10 { [200, 30, 30] } else if t < 20 { [30, 200, 30] } else { [30, 30, 200] };
            f.push(solid(f64::from(t), c));
        }
        assert_eq!(scene_changes(&f, 3.0, 8), vec![10.0, 20.0]);
        // (too close together: the strongest kept)
        assert_eq!(scene_changes(&f, 15.0, 8).len(), 1);
    }

    #[test]
    fn a_wav_header_and_silence() {
        let w = wav(&[0, 1000, -1000]);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(w.len(), 44 + 6);
        assert!(silent(&[0; 100]));
        assert!(!silent(&[5000, -5000, 5000]));
    }
}
