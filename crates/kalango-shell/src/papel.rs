//! Wallpaper: one picture per output, the version made for its proportion (design "04 · Área de trabalho
//! (responsiva)": the 16:9 scene, the tall one on portrait screens, the wide one on 21:9). Drawn by swaybg, a process
//! of its own (the picture stays while the shell restarts), started by the shell for the outputs it sees and again
//! when they change. Only this compositor's swaybg is ever replaced (a pid file per Wayland display).
//!
//! Pictures: KALANGO_PAPEL_DE_PAREDE (16:9), KALANGO_PAPEL_DE_PAREDE_ALTO, KALANGO_PAPEL_DE_PAREDE_LARGO, or the
//! installed ones in /usr/share/kalango.

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// The versions of the scene and the proportion each was drawn for (in the light theme its light version, where
/// there is one).
fn versoes() -> Vec<(f64, PathBuf)> {
    let v = |env: &str, file: &str| std::env::var_os(env).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/usr/share/kalango").join(file));
    // a picture of the person's: on every screen, cut to each one's proportion
    if let Some(p) = crate::tema::papel_escolhido().filter(|p| p.is_file()) {
        let r = gtk::gdk_pixbuf::Pixbuf::file_info(&p).map(|(_, w, h)| f64::from(w.max(1)) / f64::from(h.max(1))).unwrap_or(16.0 / 9.0);
        return vec![(r, p)];
    }
    if crate::tema::e_claro() {
        let claro = v("KALANGO_PAPEL_DE_PAREDE_CLARO", "papel-de-parede-claro.png");
        if claro.is_file() {
            return vec![(16.0 / 9.0, claro)];
        }
    }
    [
        (16.0 / 9.0, v("KALANGO_PAPEL_DE_PAREDE", "papel-de-parede.png")),
        (9.0 / 16.0, v("KALANGO_PAPEL_DE_PAREDE_ALTO", "papel-de-parede-alto.png")),
        (3840.0 / 1648.0, v("KALANGO_PAPEL_DE_PAREDE_LARGO", "papel-de-parede-largo.png")),
    ]
    .into_iter()
    .filter(|(_, p)| p.is_file())
    .collect()
}

/// KalangoOS's own scene (16:9), its dark or light version (the first use's theme cards show both).
pub fn cena(claro: bool) -> Option<PathBuf> {
    let v = |env: &str, file: &str| std::env::var_os(env).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/usr/share/kalango").join(file));
    let p = if claro { v("KALANGO_PAPEL_DE_PAREDE_CLARO", "papel-de-parede-claro.png") } else { v("KALANGO_PAPEL_DE_PAREDE", "papel-de-parede.png") };
    p.is_file().then_some(p)
}

/// The picture for an output of this size: the version whose proportion is nearest (so the crop is smallest).
pub fn para(w: i32, h: i32) -> Option<PathBuf> {
    let a = (f64::from(w.max(1)) / f64::from(h.max(1))).ln();
    versoes().into_iter().min_by(|x, y| (x.0.ln() - a).abs().total_cmp(&(y.0.ln() - a).abs())).map(|(_, p)| p)
}

fn pid_file() -> Option<PathBuf> {
    let wd = std::env::var("WAYLAND_DISPLAY").ok().filter(|w| !w.is_empty())?;
    Some(glib::user_runtime_dir().join(format!("kalango/papel-{}.pid", wd.replace('/', "_"))))
}

/// swaybg's arguments for the outputs now.
fn argumentos() -> Vec<String> {
    let mut a = Vec::new();
    let Some(d) = gdk::Display::default() else { return a };
    let ms = d.monitors();
    for m in (0..ms.n_items()).filter_map(|i| ms.item(i).and_downcast::<gdk::Monitor>()) {
        let (Some(c), g) = (m.connector(), m.geometry()) else { continue };
        if let Some(p) = para(g.width(), g.height()) {
            a.extend(["-o".into(), c.to_string(), "-i".into(), p.display().to_string(), "-m".into(), "fill".into()]);
        }
    }
    a
}

/// The process belongs to this compositor (same Wayland display in the same runtime folder): a pid file left by a
/// session that ended may point to a number now used by someone else.
fn deste_display(pid: i32) -> bool {
    let Ok(env) = std::fs::read(format!("/proc/{pid}/environ")) else { return false };
    let vars: Vec<String> = env.split(|b| *b == 0).map(|s| String::from_utf8_lossy(s).into_owned()).collect();
    let tem = |k: &str| std::env::var(k).ok().is_some_and(|v| vars.iter().any(|x| *x == format!("{k}={v}")));
    tem("WAYLAND_DISPLAY") && tem("XDG_RUNTIME_DIR")
}

fn cmdline(pid: i32) -> Option<Vec<String>> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(raw.split(|b| *b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect())
}

/// Starts swaybg with these arguments, unless the one already running for this display has them.
fn aplicar(args: &[String]) {
    if args.is_empty() {
        return;
    }
    let Some(pf) = pid_file() else { return };
    let antigo = std::fs::read_to_string(&pf).ok().and_then(|s| s.trim().parse::<i32>().ok());
    if let Some(pid) = antigo {
        match cmdline(pid) {
            Some(c) if c.first().is_some_and(|b| b.ends_with("swaybg")) && deste_display(pid) => {
                if c[1..] == *args {
                    return;
                }
                extern "C" {
                    fn kill(pid: i32, sig: i32) -> i32;
                }
                unsafe { kill(pid, 15) };
            }
            _ => {}
        }
    }
    match std::process::Command::new("swaybg").args(args).stdin(std::process::Stdio::null()).spawn() {
        Ok(c) => {
            let _ = std::fs::create_dir_all(pf.parent().unwrap_or(&pf));
            let _ = std::fs::write(&pf, c.id().to_string());
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug papel: swaybg {}", args.join(" "));
            }
            // reaped when it ends (another one replaced it), so no zombie stays behind
            std::thread::spawn(move || {
                let mut c = c;
                let _ = c.wait();
            });
        }
        Err(e) => eprintln!("kalango-shell: papel de parede (swaybg): {e}"),
    }
}

/// The theme turned light or dark: its wallpaper.
pub fn reaplicar() {
    aplicar(&argumentos());
}

/// Puts the wallpaper on every output and follows the outputs as they come, go or change size.
pub fn ligar() {
    let Some(d) = gdk::Display::default() else { return };
    // a change often comes as several notifications: applied once, a moment later
    let pendente: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    let agendar = {
        let pendente = pendente.clone();
        move || {
            if let Some(s) = pendente.borrow_mut().take() {
                s.remove();
            }
            let p = pendente.clone();
            *pendente.borrow_mut() = Some(glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                p.borrow_mut().take();
                aplicar(&argumentos());
            }));
        }
    };
    let agendar = Rc::new(agendar);
    let seguir = {
        let agendar = agendar.clone();
        move |ms: &gio::ListModel| {
            for m in (0..ms.n_items()).filter_map(|i| ms.item(i).and_downcast::<gdk::Monitor>()) {
                let a = agendar.clone();
                m.connect_geometry_notify(move |_| a());
            }
            agendar();
        }
    };
    let ms = d.monitors();
    seguir(&ms);
    ms.connect_items_changed(move |ms, _, _, _| seguir(ms));
}

#[cfg(test)]
mod tests {
    /// Each output gets the version drawn nearest its proportion.
    #[test]
    fn picks_the_version_for_the_proportion() {
        let dir = std::env::temp_dir().join(format!("kalango-papel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (env, f) in [("KALANGO_PAPEL_DE_PAREDE", "normal.png"), ("KALANGO_PAPEL_DE_PAREDE_ALTO", "alto.png"), ("KALANGO_PAPEL_DE_PAREDE_LARGO", "largo.png")] {
            std::fs::write(dir.join(f), b"x").unwrap();
            std::env::set_var(env, dir.join(f));
        }
        let nome = |w, h| super::para(w, h).and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
        assert_eq!(nome(1920, 1080), "normal.png");
        assert_eq!(nome(1366, 768), "normal.png");
        assert_eq!(nome(1600, 1200), "normal.png");
        assert_eq!(nome(1080, 1920), "alto.png");
        assert_eq!(nome(3440, 1440), "largo.png");
        assert_eq!(nome(5120, 1440), "largo.png");
        // a missing version falls back to the nearest one there is
        std::fs::remove_file(dir.join("alto.png")).unwrap();
        assert_eq!(nome(1080, 1920), "normal.png");
        for env in ["KALANGO_PAPEL_DE_PAREDE", "KALANGO_PAPEL_DE_PAREDE_ALTO", "KALANGO_PAPEL_DE_PAREDE_LARGO"] {
            std::env::remove_var(env);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
