//! kalango-tela · the screen helper of an agent's screen-control session.
//!
//! kalangod starts it as the person, in their session (the environment of the agent that asked: WAYLAND_DISPLAY,
//! NIRI_SOCKET, the session bus), writes its pid in niri's allow file (patch 0008) and talks to it over stdin and
//! stdout, one JSON request per line. It only does what it is told: every rule (which window, what is refused, how
//! many actions) is kalangod's. It never reads the person's input — the protocols it uses only send — and it ends
//! when kalangod closes its stdin or kills it.

mod atspi;
mod image;
mod keys;
mod niri;
mod wl;

use base64::Engine;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

/// The peer process of a Unix socket (SO_PEERCRED).
pub fn peer_pid(fd: i32) -> Option<i32> {
    let mut c = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let r = unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_PEERCRED, &mut c as *mut _ as *mut libc::c_void, &mut len) };
    (r == 0 && c.pid > 0).then_some(c.pid)
}

fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn num(r: &Value, k: &str) -> f64 {
    r[k].as_f64().unwrap_or(0.0)
}

/// The Wayland connection with `cap` (niri decides what a client sees when it connects: a grant written after the
/// first connection needs a new one).
fn ensure(w: &mut Option<wl::Wl>, cap: Option<&str>) {
    let ok = w.as_ref().is_some_and(|w| cap.is_none_or(|c| w.capabilities().contains(&c)));
    if !ok {
        match wl::Wl::connect() {
            Ok(n) => *w = Some(n),
            Err(e) => eprintln!("kalango-tela: {e}"),
        }
    }
}

fn handle(w: &mut Option<wl::Wl>, r: &Value) -> Result<Value, String> {
    let op = r["op"].as_str().unwrap_or("");
    let saida = r["saida"].as_str().unwrap_or("").to_string();
    let cap = match op {
        "capturar" => Some("captura"),
        "mover" | "botao" | "rolar" => Some("ponteiro"),
        "digitar" | "teclas" => Some("teclado"),
        "estado" | "sincronizar" => None,
        _ => None,
    };
    if op != "elementos" {
        ensure(w, cap);
    }
    if op == "estado" {
        let get = |what: &str| niri::request(what).unwrap_or(Value::Null);
        let wl_pid = w.as_ref().and_then(|w| w.compositor_pid());
        return Ok(json!({
            "janelas": get("Windows"), "espacos": get("Workspaces"), "saidas": get("Outputs"), "camadas": get("Layers"),
            "compositor": {"ipc": niri::compositor_pid(), "wayland": wl_pid},
            "capacidades": w.as_ref().map(|w| w.capabilities()).unwrap_or_default(),
        }));
    }
    if op == "elementos" {
        let pid = r["pid"].as_i64().unwrap_or(0) as i32;
        return atspi::elements(pid, r["titulo"].as_str().unwrap_or(""), num(r, "w") as i32, num(r, "h") as i32, r["limite"].as_u64().unwrap_or(150) as usize);
    }
    let w = w.as_mut().ok_or("no Wayland connection")?;
    match op {
        "capturar" => {
            let c = w.capture(&saida)?;
            let crop = match r.get("recorte") {
                Some(k) if k.is_object() => {
                    let x = (num(k, "x").max(0.0) as u32).min(c.w.saturating_sub(1));
                    let y = (num(k, "y").max(0.0) as u32).min(c.h.saturating_sub(1));
                    image::Crop { x, y, w: (num(k, "w") as u32).clamp(1, c.w - x), h: (num(k, "h") as u32).clamp(1, c.h - y) }
                }
                _ => image::Crop { x: 0, y: 0, w: c.w, h: c.h },
            };
            let (iw, ih) = image::fit(crop.w, crop.h, r["max_w"].as_u64().unwrap_or(1024) as u32, r["max_h"].as_u64().unwrap_or(1024) as u32);
            let rgb = image::scale(&c.data, c.stride, c.bgrx, c.y_invert, c.h, crop, iw, ih);
            let mut out = json!({"png": b64(&image::png(&rgb, iw, ih)), "w": iw, "h": ih, "quadro_w": c.w, "quadro_h": c.h, "invertido": c.y_invert});
            if let Some(mw) = r["mini_w"].as_u64() {
                // the timeline's thumbnail: the whole output, small
                let full = image::Crop { x: 0, y: 0, w: c.w, h: c.h };
                let (tw, th) = image::fit(c.w, c.h, mw as u32, mw as u32);
                let t = image::scale(&c.data, c.stride, c.bgrx, c.y_invert, c.h, full, tw, th);
                out["mini"] = json!(b64(&image::png(&t, tw, th)));
            }
            Ok(out)
        }
        "mover" => w.move_to(&saida, num(r, "x"), num(r, "y"), num(r, "w") as u32, num(r, "h") as u32).map(|_| json!({})),
        "botao" => w.button(&saida, r["codigo"].as_u64().unwrap_or(0x110) as u32, r["pressionado"].as_bool().unwrap_or(false)).map(|_| json!({})),
        "rolar" => w.scroll(&saida, r["dx"].as_i64().unwrap_or(0) as i32, r["dy"].as_i64().unwrap_or(0) as i32).map(|_| json!({})),
        "digitar" => w.type_text(r["texto"].as_str().unwrap_or("")).map(|_| json!({})),
        "teclas" => {
            let c = keys::parse_combo(r["combo"].as_str().unwrap_or(""))?;
            w.combo(&c).map(|_| json!({}))
        }
        "sincronizar" => w.roundtrip().map(|_| json!({})),
        _ => Err(format!("unknown op {op:?}")),
    }
}

fn main() {
    // (connected on the first request: kalangod writes the grant after starting this process)
    let mut w: Option<wl::Wl> = None;
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let Ok(r) = serde_json::from_str::<Value>(&line) else { continue };
        let reply = match handle(&mut w, &r) {
            Ok(mut v) => {
                v["ok"] = json!(true);
                v
            }
            Err(e) => json!({"ok": false, "erro": e}),
        };
        if writeln!(out, "{reply}").and_then(|_| out.flush()).is_err() {
            break;
        }
    }
}
