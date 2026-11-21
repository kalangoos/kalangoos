//! KalangoOS's system sounds: the sound theme's file for an event ID (freedesktop Sound Theme and Sound Naming
//! specifications, `<theme>/stereo/<id>.oga`), decoded once (symphonia, in Rust) and kept, then handed to a short
//! GStreamer pipeline (appsrc → audioconvert → audioresample → volume → pipewiresink) that PipeWire plays as an event
//! sound and that ends with the sound — inside the program that plays it, no process per sound. The shell and the
//! login screen use it; when each sound plays, and at which volume, is theirs to say.
//!
//! `KALANGO_SONS_DESTINO` names the PipeWire node the sounds go to (a null sink when testing or measuring).

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// KalangoOS's own sound theme (`/usr/share/sounds/kalango`).
pub const OWN_THEME: &str = "kalango";
const FALLBACK_THEME: &str = "freedesktop";
const EXTENSIONS: [&str; 3] = ["oga", "ogg", "wav"];
/// Decoded sounds kept in memory at most (16-bit PCM: a 1 s stereo sound at 48 kHz is 188 KiB).
const CACHE_BYTES: usize = 3 << 20;
/// The session's sounds play once each: not worth keeping.
const NOT_KEPT: [&str; 4] = ["system-bootup", "desktop-login", "desktop-logout", "system-shutdown"];

/// An event ID and its fallbacks (Sound Naming spec: the last `-part` goes, "dialog-question" → "dialog").
fn names(id: &str) -> Vec<&str> {
    let mut v = vec![id];
    let mut s = id;
    while let Some(i) = s.rfind('-') {
        s = &s[..i];
        v.push(s);
    }
    v
}

/// The file for an event ID: in `theme`, then KalangoOS's own theme, then `freedesktop`; in each, in `bases` in
/// order. Only plain IDs (letters, digits and `-`): nothing outside the themes is read.
pub fn resolve_in(id: &str, theme: &str, bases: &[PathBuf]) -> Option<PathBuf> {
    let plain = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') && !s.starts_with('-');
    if !plain(id) {
        return None;
    }
    let mut themes = vec![theme];
    for t in [OWN_THEME, FALLBACK_THEME] {
        if !themes.contains(&t) {
            themes.push(t);
        }
    }
    themes.retain(|t| plain(t));
    for name in names(id) {
        for t in &themes {
            for b in bases {
                for ext in EXTENSIONS {
                    let p = b.join(t).join("stereo").join(format!("{name}.{ext}"));
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

/// A decoded sound: 16-bit interleaved PCM.
#[derive(Debug)]
pub struct Pcm {
    pub rate: u32,
    pub channels: u32,
    pub samples: Vec<i16>,
}

impl Pcm {
    pub fn length(&self) -> Duration {
        let frames = self.samples.len() as u64 / self.channels.max(1) as u64;
        Duration::from_micros(frames * 1_000_000 / self.rate.max(1) as u64)
    }
}

fn open(path: &Path) -> Result<Box<dyn symphonia::core::formats::FormatReader>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(if ext == "oga" { "ogg" } else { ext });
    }
    symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).map(|p| p.format).map_err(|e| e.to_string())
}

/// The whole file as PCM.
pub fn decode(path: &Path) -> Result<Pcm, String> {
    let mut format = open(path)?;
    let track = format.default_track().ok_or("no audio")?.clone();
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).map_err(|e| e.to_string())?;
    let mut pcm = Pcm { rate: track.codec_params.sample_rate.unwrap_or(48_000), channels: track.codec_params.channels.map(|c| c.count() as u32).unwrap_or(2), samples: Vec::new() };
    let mut buf: Option<SampleBuffer<i16>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id() != track.id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(a) => {
                let spec = *a.spec();
                pcm.rate = spec.rate;
                pcm.channels = spec.channels.count() as u32;
                let b = match buf.as_mut() {
                    Some(b) if b.capacity() >= a.capacity() * spec.channels.count() => b,
                    _ => buf.insert(SampleBuffer::new(a.capacity() as u64, spec)),
                };
                b.copy_interleaved_ref(a);
                pcm.samples.extend_from_slice(b.samples());
            }
            // (a damaged packet is skipped, as players do)
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    if pcm.samples.is_empty() {
        return Err("no audio".into());
    }
    Ok(pcm)
}

/// How long a sound file plays (read from its header, not decoded).
pub fn file_length(path: &Path) -> Option<Duration> {
    let format = open(path).ok()?;
    let p = &format.default_track()?.codec_params;
    let frames = p.n_frames?;
    Some(Duration::from_micros(frames * 1_000_000 / p.sample_rate? as u64))
}

/// A decoded sound ready to push: its caps and the buffers of 1024 frames each (PipeWire takes a quantum at a time;
/// one buffer with the whole sound went out silent), timestamped.
#[derive(Clone)]
struct Prepared {
    caps: gst::Caps,
    buffers: Vec<gst::Buffer>,
    length: Duration,
}

const CHUNK_FRAMES: usize = 1024;

type Cache = (HashMap<PathBuf, Prepared>, Vec<(PathBuf, usize)>);

/// The sounds decoded (by file), and their sizes oldest first.
fn cache() -> &'static Mutex<Cache> {
    static C: OnceLock<Mutex<Cache>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn to_gst(pcm: &Pcm) -> Prepared {
    let ch = pcm.channels.max(1) as usize;
    let ns = |frames: usize| frames as u64 * 1_000_000_000 / pcm.rate.max(1) as u64;
    let buffers = pcm.samples.chunks(CHUNK_FRAMES * ch).scan(0usize, |at, c| {
        let bytes: Vec<u8> = c.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut b = gst::Buffer::from_mut_slice(bytes);
        {
            let m = b.get_mut().expect("a new buffer is writable");
            let frames = c.len() / ch;
            m.set_pts(gst::ClockTime::from_nseconds(ns(*at)));
            m.set_duration(gst::ClockTime::from_nseconds(ns(*at + frames) - ns(*at)));
            m.set_offset(*at as u64);
            *at += frames;
        }
        Some(b)
    }).collect();
    let caps = gst::Caps::builder("audio/x-raw").field("format", "S16LE").field("layout", "interleaved")
        .field("rate", pcm.rate as i32).field("channels", pcm.channels as i32).build();
    Prepared { caps, buffers, length: pcm.length() }
}

/// The sound ready to push: from the cache, or decoded now (and kept, unless it plays once a session).
fn prepared(id: &str, path: &Path) -> Result<Prepared, String> {
    if let Some(p) = cache().lock().ok().and_then(|c| c.0.get(path).cloned()) {
        return Ok(p);
    }
    let pcm = decode(path)?;
    let p = to_gst(&pcm);
    if !NOT_KEPT.contains(&id) {
        if let Ok(mut k) = cache().lock() {
            let (map, order) = &mut *k;
            map.insert(path.to_path_buf(), p.clone());
            order.push((path.to_path_buf(), pcm.samples.len() * 2));
            while order.iter().map(|(_, n)| n).sum::<usize>() > CACHE_BYTES && order.len() > 1 {
                let (old, _) = order.remove(0);
                map.remove(&old);
            }
        }
    }
    Ok(p)
}

fn sink() -> Result<gst::Element, String> {
    match gst::ElementFactory::make("pipewiresink").build() {
        Ok(s) => {
            let props = gst::Structure::builder("props").field("media.role", "Notification").field("media.category", "Playback")
                .field("application.name", "KalangoOS").field("node.name", "kalango-sons").build();
            s.set_property("stream-properties", props);
            s.set_property("client-name", "KalangoOS");
            if let Some(t) = std::env::var("KALANGO_SONS_DESTINO").ok().filter(|t| !t.is_empty()) {
                s.set_property("target-object", t);
            }
            Ok(s)
        }
        Err(_) => gst::ElementFactory::make("autoaudiosink").build().map_err(|e| e.to_string()),
    }
}

/// Plays one sound to its end (in the calling thread: `play` gives each its own).
pub fn play_blocking(id: &str, path: &Path, volume: f64) -> Result<(), String> {
    gst::init().map_err(|e| e.to_string())?;
    let p = prepared(id, path)?;
    let pipe = gst::Pipeline::new();
    let src = gst_app::AppSrc::builder().caps(&p.caps).format(gst::Format::Time).max_bytes(0).block(false).build();
    let els = [
        src.upcast_ref::<gst::Element>().clone(),
        gst::ElementFactory::make("audioconvert").build().map_err(|e| e.to_string())?,
        gst::ElementFactory::make("audioresample").build().map_err(|e| e.to_string())?,
        gst::ElementFactory::make("volume").property("volume", volume.clamp(0.0, 1.0)).build().map_err(|e| e.to_string())?,
        sink()?,
    ];
    pipe.add_many(&els).map_err(|e| e.to_string())?;
    gst::Element::link_many(&els).map_err(|e| e.to_string())?;
    // (the buffers share the cached memory: nothing is copied)
    for b in &p.buffers {
        src.push_buffer(b.clone()).map_err(|e| format!("{e:?}"))?;
    }
    let _ = src.end_of_stream();
    pipe.set_state(gst::State::Playing).map_err(|e| e.to_string())?;
    let bus = pipe.bus().ok_or("no bus")?;
    let mut r = Ok(());
    // (never longer than the sound and a little: a device that went away does not keep the thread)
    let limit = gst::ClockTime::from_nseconds((p.length + Duration::from_secs(3)).as_nanos() as u64);
    match bus.timed_pop_filtered(limit, &[gst::MessageType::Eos, gst::MessageType::Error]) {
        Some(m) => {
            if let gst::MessageView::Error(e) = m.view() {
                r = Err(e.error().to_string());
            }
        }
        None => r = Err("timed out".into()),
    }
    let _ = pipe.set_state(gst::State::Null);
    r
}

/// Plays the sound in `path` for this event ID at this volume (0 – 1, linear: the same scale libcanberra's `-V` got
/// in decibels) in its own thread (GStreamer starts there, the first time: never before a sound, never in the caller's
/// thread); `program` names who says so when it fails. How long it plays, when it started.
pub fn play(id: &str, path: &Path, volume: f64, program: &'static str) -> Option<Duration> {
    let len = file_length(path).unwrap_or_default();
    let (id, path) = (id.to_string(), path.to_path_buf());
    std::thread::Builder::new().name("kalango-som".into()).spawn(move || {
        if let Err(e) = play_blocking(&id, &path, volume) {
            // (once a process: without a sound server — the login screen's user may have none — every sound fails
            // the same way, in a few milliseconds, and the log says it one time)
            static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
                eprintln!("{program}: som {id}: {e} (os próximos erros de som não são repetidos)");
            }
        }
    }).ok().map(|_| len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme_dir() -> PathBuf {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/sounds")).to_path_buf()
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kalango-sons-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn put(base: &Path, theme: &str, name: &str) -> PathBuf {
        let p = base.join(theme).join("stereo").join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"x").unwrap();
        p
    }


    #[test]
    fn user_theme_first_then_system_then_own_then_freedesktop() {
        let (user, system) = (temp("user"), temp("system"));
        let bases = [user.clone(), system.clone()];
        assert_eq!(resolve_in("bell", "outro", &bases), None);
        let fd = put(&system, "freedesktop", "bell.oga");
        assert_eq!(resolve_in("bell", "outro", &bases), Some(fd));
        let own = put(&system, "kalango", "bell.oga");
        assert_eq!(resolve_in("bell", "outro", &bases), Some(own));
        let sys = put(&system, "outro", "bell.oga");
        assert_eq!(resolve_in("bell", "outro", &bases), Some(sys));
        let mine = put(&user, "outro", "bell.ogg");
        assert_eq!(resolve_in("bell", "outro", &bases), Some(mine));
        let _ = (std::fs::remove_dir_all(&user), std::fs::remove_dir_all(&system));
    }

    #[test]
    fn names_fall_back_as_the_naming_spec_says_and_nothing_outside_the_themes() {
        let base = temp("fallback");
        let dialog = put(&base, "freedesktop", "dialog.oga");
        assert_eq!(resolve_in("dialog-question", "kalango", std::slice::from_ref(&base)), Some(dialog));
        assert_eq!(names("kalango-agent-done"), ["kalango-agent-done", "kalango-agent", "kalango"]);
        put(&base, "kalango", "x.oga");
        for bad in ["../kalango/stereo/x", "a/b", "", "-x", "x.oga"] {
            assert_eq!(resolve_in(bad, "kalango", std::slice::from_ref(&base)), None, "{bad}");
        }
        assert_eq!(resolve_in("bell", "../../etc", std::slice::from_ref(&base)), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn decodes_the_theme_s_sounds() {
        let p = theme_dir().join("kalango/stereo/message-new-instant.oga");
        let pcm = decode(&p).unwrap();
        assert_eq!((pcm.rate, pcm.channels), (48_000, 2));
        let len = pcm.length().as_millis();
        assert!((1050..=1150).contains(&len), "{len} ms");
        assert!(pcm.samples.iter().any(|s| s.unsigned_abs() > 10_000), "silêncio");
        let header = file_length(&p).unwrap().as_millis();
        assert!(header.abs_diff(len) <= 30, "{header} vs {len}");
        assert_eq!(file_length(&theme_dir().join("kalango/stereo/system-bootup.oga")).map(|d| d.as_millis() / 100), Some(40));
    }

    /// Plays into a PipeWire null sink (never the speakers): KALANGO_SONS_TESTE_DESTINO names it, e.g.
    /// `pw-cli create-node adapter '{ factory.name=support.null-audio-sink node.name=kalango-teste-nulo
    /// media.class=Audio/Sink object.linger=true }'`. Without it the test says so and passes.
    #[test]
    fn plays_to_its_end_in_a_null_sink() {
        let Some(dest) = std::env::var("KALANGO_SONS_TESTE_DESTINO").ok().filter(|d| !d.is_empty()) else {
            eprintln!("(sem KALANGO_SONS_TESTE_DESTINO: não tocado)");
            return;
        };
        std::env::set_var("KALANGO_SONS_DESTINO", dest);
        let p = theme_dir().join("kalango/stereo/screen-capture.oga");
        let t = std::time::Instant::now();
        play_blocking("screen-capture", &p, 0.4).unwrap();
        let took = t.elapsed();
        assert!(took >= Duration::from_millis(300) && took < Duration::from_secs(3), "{took:?}");
        // the second time it comes from the cache
        assert!(cache().lock().unwrap().0.contains_key(&p));
        play_blocking("screen-capture", &p, 0.4).unwrap();
    }

    #[test]
    fn a_sound_goes_in_quantum_sized_buffers_one_after_another() {
        gst::init().unwrap();
        let p = theme_dir().join("kalango/stereo/message-new-instant.oga");
        let pcm = decode(&p).unwrap();
        let g = to_gst(&pcm);
        // (one buffer with the whole sound reached PipeWire as silence)
        assert!(g.buffers.len() > 40, "{}", g.buffers.len());
        assert!(g.buffers.iter().all(|b| b.size() <= CHUNK_FRAMES * 4));
        let mut next = gst::ClockTime::ZERO;
        for b in &g.buffers {
            assert_eq!(b.pts(), Some(next));
            next += b.duration().unwrap();
        }
        assert!(next.mseconds().abs_diff(pcm.length().as_millis() as u64) <= 1);
    }
}
