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
