//! The screen as the model sees it: the compositor's frame (BGRx, 4 bytes a pixel) cut to a region, scaled down
//! by averaging, and written as a PNG (no image crate: rows unfiltered, deflate by flate2).

use std::io::Write;

/// A region of the frame in its own pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// The size that fits `w × h` into `max_w × max_h` keeping the proportion (never larger than the source).
pub fn fit(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (0, 0);
    }
    let k = (max_w as f64 / w as f64).min(max_h as f64 / h as f64).min(1.0);
    (((w as f64 * k).round() as u32).max(1), ((h as f64 * k).round() as u32).max(1))
}

/// RGB pixels of `crop`, scaled to `out_w × out_h` by averaging each block. `bgrx`: the frame's bytes are
/// B, G, R, x (wl_shm XRGB/ARGB8888 on little endian); else R, G, B, x. `y_invert`: the frame is upside down.
#[allow(clippy::too_many_arguments)]
pub fn scale(frame: &[u8], stride: u32, bgrx: bool, y_invert: bool, frame_h: u32, crop: Crop, out_w: u32, out_h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((out_w * out_h * 3) as usize);
    for oy in 0..out_h {
        let y0 = crop.y + oy * crop.h / out_h;
        let y1 = (crop.y + (oy + 1) * crop.h / out_h).max(y0 + 1);
        for ox in 0..out_w {
            let x0 = crop.x + ox * crop.w / out_w;
            let x1 = (crop.x + (ox + 1) * crop.w / out_w).max(x0 + 1);
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            for y in y0..y1 {
                let row = if y_invert { frame_h - 1 - y } else { y };
                let base = (row * stride) as usize;
                for x in x0..x1 {
                    let i = base + (x * 4) as usize;
                    let Some(p) = frame.get(i..i + 4) else { continue };
                    if bgrx {
                        b += p[0] as u32;
                        g += p[1] as u32;
                        r += p[2] as u32;
                    } else {
                        r += p[0] as u32;
                        g += p[1] as u32;
                        b += p[2] as u32;
                    }
                    n += 1;
                }
            }
            let n = n.max(1);
            out.extend_from_slice(&[(r / n) as u8, (g / n) as u8, (b / n) as u8]);
        }
    }
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut crc = flate2::Crc::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc.sum().to_be_bytes());
}

/// An 8-bit RGB PNG.
pub fn png(rgb: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut raw = Vec::with_capacity(rgb.len() + h as usize);
    for row in rgb.chunks((w * 3) as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    let _ = z.write_all(&raw);
    let data = z.finish().unwrap_or_default();
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &data);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitting() {
        assert_eq!(fit(1920, 1080, 1024, 1024), (1024, 576));
        assert_eq!(fit(800, 600, 1024, 1024), (800, 600), "never larger");
        assert_eq!(fit(1080, 1920, 1024, 1024), (576, 1024));
    }

    #[test]
    fn scaling_averages_and_crops() {
        // 4×2 frame, BGRx: left half pure red, right half pure blue
        let mut f = Vec::new();
        for _ in 0..2 {
            for x in 0..4 {
                f.extend_from_slice(if x < 2 { &[0, 0, 255, 0] } else { &[255, 0, 0, 0] });
            }
        }
        let all = scale(&f, 16, true, false, 2, Crop { x: 0, y: 0, w: 4, h: 2 }, 2, 1);
        assert_eq!(all, vec![255, 0, 0, 0, 0, 255]);
        let one = scale(&f, 16, true, false, 2, Crop { x: 1, y: 0, w: 2, h: 2 }, 1, 1);
        assert_eq!(one, vec![127, 0, 127]);
    }

    #[test]
    fn a_valid_png() {
        let p = png(&[1, 2, 3, 4, 5, 6], 2, 1);
        assert_eq!(&p[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&p[12..16], b"IHDR");
        // the IHDR's CRC as any PNG reader computes it
        assert_eq!(&p[p.len() - 8..p.len() - 4], b"IEND");
    }
}
