//! niri's IPC (the socket in NIRI_SOCKET): the windows, workspaces, outputs and layer surfaces, as JSON.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

pub fn socket() -> Option<String> {
    std::env::var("NIRI_SOCKET").ok().filter(|s| !s.is_empty())
}

/// One request ("Windows", "Layers"…): its answer's value.
pub fn request(what: &str) -> Result<Value, String> {
    let path = socket().ok_or("NIRI_SOCKET is not set")?;
    let mut s = UnixStream::connect(&path).map_err(|e| format!("niri: {e}"))?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(3))).ok();
    writeln!(s, "\"{what}\"").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(&s).read_line(&mut line).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&line).map_err(|e| format!("niri: {e}"))?;
    match v.get("Ok") {
        Some(ok) => Ok(ok.get(what).cloned().unwrap_or(Value::Null)),
        None => Err(format!("niri: {}", v.get("Err").and_then(Value::as_str).unwrap_or("error"))),
    }
}

/// The compositor's process (the peer of the IPC socket).
pub fn compositor_pid() -> Option<i32> {
    use std::os::fd::AsRawFd;
    let s = UnixStream::connect(socket()?).ok()?;
    crate::peer_pid(s.as_raw_fd())
}
