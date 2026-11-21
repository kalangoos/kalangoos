//! Measuring and checking by hand (not shipped): `cargo run -p kalango-sons --example tocar -- <file.oga> [volume]`
//! plays one sound to its end in this thread and says how long it took and how it ended.
fn main() {
    let mut a = std::env::args().skip(1);
    let path = std::path::PathBuf::from(a.next().expect("arquivo"));
    let volume: f64 = a.next().and_then(|v| v.parse().ok()).unwrap_or(0.4);
    let t = std::time::Instant::now();
    let r = kalango_sons::play_blocking("tocar", &path, volume);
    eprintln!("{:?} em {} ms", r, t.elapsed().as_millis());
}
