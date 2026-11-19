//! The Music app's library: the audio files in the person's Music folder, their tags (title, artist, album, genre),
//! length and cover (read by symphonia, in Rust), the favourites, and each track's waveform — the real loudness of
//! the track, in 96 bars, kept in a cache once read.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey};
use symphonia::core::probe::Hint;

pub const AUDIO: &[&str] = &["mp3", "flac", "ogg", "oga", "m4a", "mp4", "aac", "wav", "alac"];
pub const BARS: usize = 96;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Track {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub seconds: u64,
    /// the cover, saved from the file's own picture (none: the app draws one)
    pub cover: Option<PathBuf>,
    /// when the file last changed (a changed file is read again)
    pub mtime: u64,
}

impl Track {
    pub fn label(&self) -> String {
        if self.title.is_empty() { self.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default() } else { self.title.clone() }
    }
}

/// The Music folder: `KALANGO_MUSICA` (the session test), else the system's Music folder.
pub fn music_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("KALANGO_MUSICA").filter(|v| !v.is_empty()) {
        return PathBuf::from(d);
    }
    crate::pastas::especial(gtk::glib::UserDirectory::Music, "Música")
}

fn cache_dir() -> PathBuf {
    gtk::glib::user_cache_dir().join("kalango/musica")
}

fn data_dir() -> PathBuf {
    gtk::glib::user_data_dir().join("kalango/musica")
}

fn mtime(p: &Path) -> u64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

fn short_hash(s: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn open(p: &Path) -> Option<symphonia::core::probe::ProbeResult> {
    let file = std::fs::File::open(p).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(e) = p.extension().and_then(|e| e.to_str()) {
        hint.with_extension(e);
    }
    symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).ok()
}

/// One file's tags, length and cover.
pub fn read_track(p: &Path) -> Option<Track> {
    let mut probed = open(p)?;
    let mut t = Track { path: p.to_path_buf(), mtime: mtime(p), ..Default::default() };
    let mut cover: Option<(Vec<u8>, String)> = None;
    let mut take = |rev: &MetadataRevision, t: &mut Track| {
        for tag in rev.tags() {
            let v = tag.value.to_string();
            match tag.std_key {
                Some(StandardTagKey::TrackTitle) if t.title.is_empty() => t.title = v,
                Some(StandardTagKey::Artist) if t.artist.is_empty() => t.artist = v,
                Some(StandardTagKey::AlbumArtist) if t.artist.is_empty() => t.artist = v,
                Some(StandardTagKey::Album) if t.album.is_empty() => t.album = v,
                Some(StandardTagKey::Genre) if t.genre.is_empty() => t.genre = v,
                _ => {}
            }
        }
        if cover.is_none() {
            if let Some(v) = rev.visuals().first() {
                cover = Some((v.data.to_vec(), v.media_type.clone()));
            }
        }
    };
    if let Some(m) = probed.metadata.get() {
        if let Some(rev) = m.current() {
            take(rev, &mut t);
        }
    }
    let format = &mut probed.format;
    if let Some(rev) = format.metadata().current() {
        take(rev, &mut t);
    }
    if let Some(track) = format.default_track() {
        let cp = &track.codec_params;
        t.seconds = match (cp.n_frames, cp.sample_rate, cp.time_base) {
            (Some(n), _, Some(tb)) => tb.calc_time(n).seconds,
            (Some(n), Some(r), None) if r > 0 => n / u64::from(r),
            _ => 0,
        };
    }
    if let Some((data, kind)) = cover {
        let ext = if kind.contains("png") { "png" } else { "jpg" };
        let dir = cache_dir().join("capas");
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join(format!("{}.{ext}", short_hash(&p.display().to_string())));
        if std::fs::write(&f, data).is_ok() {
            t.cover = Some(f);
        }
    }
    Some(t)
}

/// The library: the Music folder's tracks (the cache kept for files that did not change), by artist, album and title.
pub fn scan() -> Vec<Track> {
    let cache_file = cache_dir().join("biblioteca.json");
    let cached: Vec<Track> = std::fs::read_to_string(&cache_file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let mut files = Vec::new();
    let mut stack = vec![music_dir()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| AUDIO.contains(&x.to_lowercase().as_str())) {
                files.push(p);
            }
        }
    }
    let mut out: Vec<Track> = files.into_iter().filter_map(|p| {
        let m = mtime(&p);
        cached.iter().find(|t| t.path == p && t.mtime == m).cloned().or_else(|| read_track(&p))
    }).collect();
    out.sort_by_key(|t| (t.artist.to_lowercase(), t.album.to_lowercase(), t.label().to_lowercase()));
    let _ = std::fs::create_dir_all(cache_dir());
    let _ = std::fs::write(&cache_file, serde_json::to_string(&out).unwrap_or_default());
    out
}

/// A track's waveform: its peaks in [`BARS`] bars (0 to 1), decoded once and then read from the cache.
pub fn waveform(t: &Track) -> Vec<f32> {
    let f = cache_dir().join("ondas").join(format!("{}-{}.json", short_hash(&t.path.display().to_string()), t.mtime));
    if let Some(v) = std::fs::read_to_string(&f).ok().and_then(|s| serde_json::from_str::<Vec<f32>>(&s).ok()) {
        return v;
    }
    let v = decode_peaks(&t.path).unwrap_or_else(|| vec![0.3; BARS]);
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&f, serde_json::to_string(&v).unwrap_or_default());
    v
}

fn decode_peaks(p: &Path) -> Option<Vec<f32>> {
    let probed = open(p)?;
    let mut format = probed.format;
    let track = format.default_track()?;
    let id = track.id;
    let total = track.codec_params.n_frames.unwrap_or(0).max(1);
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).ok()?;
    let mut peaks = vec![0f32; BARS];
    let mut at: u64 = 0;
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != id {
            continue;
        }
        let Ok(buf) = decoder.decode(&packet) else { continue };
        let spec = *buf.spec();
        let frames = buf.frames() as u64;
        let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
        sb.copy_interleaved_ref(buf);
        let ch = spec.channels.count().max(1);
        for (i, frame) in sb.samples().chunks(ch).enumerate() {
            let b = (((at + i as u64) * BARS as u64) / total).min(BARS as u64 - 1) as usize;
            let s = frame.iter().fold(0f32, |m, x| m.max(x.abs()));
            if s > peaks[b] {
                peaks[b] = s;
            }
        }
        at += frames;
    }
    // (the loudest bar is full height)
    let max = peaks.iter().cloned().fold(0f32, f32::max).max(1e-6);
    Some(peaks.into_iter().map(|x| (x / max).clamp(0.05, 1.0)).collect())
}

// ------------------------------------------------------------------------------------------------- favourites

fn favourites_file() -> PathBuf {
    data_dir().join("favoritas.json")
}

pub fn favourites() -> Vec<PathBuf> {
    std::fs::read_to_string(favourites_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn set_favourite(p: &Path, on: bool) {
    let mut v = favourites();
    v.retain(|x| x != p);
    if on {
        v.insert(0, p.to_path_buf());
    }
    let _ = std::fs::create_dir_all(data_dir());
    let _ = std::fs::write(favourites_file(), serde_json::to_string_pretty(&v).unwrap_or_default());
}

/// The mixes the agent made and the person kept (name, line, tracks).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Mix {
    pub title: String,
    pub line: String,
    pub tracks: Vec<PathBuf>,
}

fn mixes_file() -> PathBuf {
    data_dir().join("listas.json")
}

pub fn mixes() -> Vec<Mix> {
    std::fs::read_to_string(mixes_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn keep_mix(m: &Mix) {
    let mut v = mixes();
    v.retain(|x| x.title != m.title);
    v.insert(0, m.clone());
    let _ = std::fs::create_dir_all(data_dir());
    let _ = std::fs::write(mixes_file(), serde_json::to_string_pretty(&v).unwrap_or_default());
}

/// "3:24"
pub fn clock(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A WAV file: one second of a 440 Hz tone that grows louder, then silence.
    fn wav(path: &Path) {
        let rate = 8000u32;
        let n = rate as usize * 2;
        let mut data = Vec::with_capacity(n * 2);
        for i in 0..n {
            let amp = if i < rate as usize { (i as f32 / rate as f32) * 0.8 } else { 0.0 };
            let s = (amp * (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * i16::MAX as f32) as i16;
            data.extend_from_slice(&s.to_le_bytes());
        }
        let mut f = Vec::new();
        f.extend_from_slice(b"RIFF");
        f.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        f.extend_from_slice(b"WAVEfmt ");
        f.extend_from_slice(&16u32.to_le_bytes());
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&rate.to_le_bytes());
        f.extend_from_slice(&(rate * 2).to_le_bytes());
        f.extend_from_slice(&2u16.to_le_bytes());
        f.extend_from_slice(&16u16.to_le_bytes());
        f.extend_from_slice(b"data");
        f.extend_from_slice(&(data.len() as u32).to_le_bytes());
        f.extend_from_slice(&data);
        std::fs::write(path, f).unwrap();
    }

    #[test]
    fn a_track_is_read_and_its_waveform_follows_its_loudness() {
        let dir = std::env::temp_dir().join(format!("kalango-musica-teste-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("tom.wav");
        wav(&p);
        let t = read_track(&p).expect("read");
        assert_eq!(t.seconds, 2);
        assert_eq!(t.label(), "tom");
        let w = decode_peaks(&p).expect("peaks");
        assert_eq!(w.len(), BARS);
        // louder towards the middle, silent in the second half
        assert!(w[BARS / 2 - 2] > w[4] && w[BARS - 2] <= 0.06, "{w:?}");
        assert_eq!(clock(204), "3:24");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
