//! The image viewer's edits, on plain RGBA pixels (a background thread can run them): the background out (from the
//! edges inward, the colour the edges have), sharper, the adjustments, and colour and tone variations.

/// RGBA pixels, row after row (no padding).
#[derive(Clone)]
pub struct Rgba {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

fn dist(a: &[u8], b: &[u8]) -> f64 {
    let (dr, dg, db) = (a[0] as f64 - b[0] as f64, a[1] as f64 - b[1] as f64, a[2] as f64 - b[2] as f64);
    (dr * dr + dg * dg + db * db).sqrt()
}

/// The background out: from every edge pixel, the region of colours close to it (and to its neighbour) becomes
/// transparent — the background a product photo, an illustration or a screenshot has. A pixel next to the cleared
/// region that is still near its colour gets half alpha (a soft edge). Returns the pixels cleared.
pub fn remove_background(img: &mut Rgba, tolerance: f64) -> usize {
    let (w, h) = (img.width, img.height);
    if w == 0 || h == 0 {
        return 0;
    }
    let px = |i: usize| i * 4;
    let mut seen = vec![false; w * h];
    let mut cleared = vec![false; w * h];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    // (the edges' seeds: each one grows into pixels close to the colour it came from)
    for x in 0..w {
        stack.push((x, 0));
        stack.push((x, h - 1));
    }
    for y in 0..h {
        stack.push((0, y));
        stack.push((w - 1, y));
    }
    // the edge colour each region started from
    let mut origin: Vec<u32> = vec![u32::MAX; w * h];
    for &(x, y) in &stack {
        origin[y * w + x] = (y * w + x) as u32;
    }
    let mut n = 0;
    while let Some((x, y)) = stack.pop() {
        let i = y * w + x;
        if seen[i] {
            continue;
        }
        seen[i] = true;
        let o = origin[i] as usize;
        let here = &img.data[px(i)..px(i) + 4];
        let seed = &img.data[px(o)..px(o) + 4];
        // (an already transparent pixel belongs to the background too)
        if here[3] > 16 && dist(here, seed) > tolerance {
            continue;
        }
        cleared[i] = true;
        n += 1;
        let mut push = |nx: usize, ny: usize| {
            let j = ny * w + nx;
            if !seen[j] {
                origin[j] = o as u32;
                stack.push((nx, ny));
            }
        };
        if x > 0 {
            push(x - 1, y);
        }
        if x + 1 < w {
            push(x + 1, y);
        }
        if y > 0 {
            push(x, y - 1);
        }
        if y + 1 < h {
            push(x, y + 1);
        }
    }
    // the soft edge: kept pixels touching the cleared region, close to its colour, become half transparent
    let mut soft = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if cleared[i] {
                continue;
            }
            let near = [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)].into_iter()
                .filter(|&(a, b)| a < w && b < h).find(|&(a, b)| cleared[b * w + a]);
            if let Some((a, b)) = near {
                let j = b * w + a;
                if dist(&img.data[px(i)..px(i) + 4], &img.data[px(j)..px(j) + 4]) < tolerance * 2.0 {
                    soft.push(i);
                }
            }
        }
    }
    for i in 0..w * h {
        if cleared[i] {
            img.data[px(i) + 3] = 0;
        }
    }
    for i in soft {
        img.data[px(i) + 3] = img.data[px(i) + 3].min(128);
    }
    n
}

/// Sharper: an unsharp mask (a 3 × 3 blur taken away, `amount` of the difference added).
pub fn sharpen(img: &Rgba, amount: f64) -> Rgba {
    let (w, h) = (img.width, img.height);
    let mut out = img.clone();
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                let mut sum = 0.0;
                let mut k = 0.0;
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                        if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                            sum += img.data[(ny as usize * w + nx as usize) * 4 + c] as f64;
                            k += 1.0;
                        }
                    }
                }
                let v = img.data[(y * w + x) * 4 + c] as f64;
                let blur = sum / k;
                out.data[(y * w + x) * 4 + c] = (v + amount * (v - blur)).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// Brightness and contrast (-1 to 1, 0 is as it is) and saturation (0 grey, 1 as it is, 2 twice).
pub fn adjust(img: &Rgba, brightness: f64, contrast: f64, saturation: f64) -> Rgba {
    let mut out = img.clone();
    let c = 1.0 + contrast;
    for p in out.data.chunks_exact_mut(4) {
        let (r, g, b) = (p[0] as f64, p[1] as f64, p[2] as f64);
        let grey = 0.299 * r + 0.587 * g + 0.114 * b;
        for (i, v) in [r, g, b].into_iter().enumerate() {
            let s = grey + (v - grey) * saturation;
            let k = (s - 128.0) * c + 128.0 + brightness * 128.0;
            p[i] = k.round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// Warmer (+) or cooler (−): red up and blue down, or the other way.
pub fn temperature(img: &Rgba, t: f64) -> Rgba {
    let mut out = img.clone();
    for p in out.data.chunks_exact_mut(4) {
        p[0] = (p[0] as f64 + 28.0 * t).round().clamp(0.0, 255.0) as u8;
        p[2] = (p[2] as f64 - 28.0 * t).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// Four variations of colour and tone: warm, cool, black and white, vivid (name, pixels).
pub fn variations(img: &Rgba) -> Vec<(&'static str, Rgba)> {
    vec![
        ("quente", temperature(img, 1.0)),
        ("fria", temperature(img, -1.0)),
        ("pb", adjust(img, 0.0, 0.1, 0.0)),
        ("viva", adjust(img, 0.02, 0.15, 1.45)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 4]) -> Rgba {
        let mut data = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&f(x, y));
            }
        }
        Rgba { width: w, height: h, data }
    }

    #[test]
    fn the_background_from_the_edges_goes_and_the_object_stays() {
        // a dark background with a red square in the middle
        let mut img = picture(20, 20, |x, y| if (6..14).contains(&x) && (6..14).contains(&y) { [200, 30, 40, 255] } else { [12, 13, 16, 255] });
        let n = remove_background(&mut img, 30.0);
        assert_eq!(n, 400 - 64);
        assert_eq!(img.data[3], 0);
        assert_eq!(img.data[(10 * 20 + 10) * 4 + 3], 255);
    }

    #[test]
    fn edits_keep_the_size_and_change_the_pixels() {
        let img = picture(8, 8, |x, _| if x < 4 { [100, 100, 100, 255] } else { [150, 120, 90, 255] });
        let s = sharpen(&img, 1.0);
        assert_eq!(s.data.len(), img.data.len());
        assert!(s.data[(3) * 4] < 100 && s.data[4 * 4] > 150, "the edge gets stronger");
        let grey = adjust(&img, 0.0, 0.0, 0.0);
        assert_eq!(grey.data[4 * 4], grey.data[4 * 4 + 1]);
        assert_eq!(variations(&img).len(), 4);
        assert!(temperature(&img, 1.0).data[0] > 100 && temperature(&img, 1.0).data[2] < 100);
    }
}
