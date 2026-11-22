//! Dictation shared by the agent composer and the command bar: records with PipeWire (or KALANGO_GRAVADOR in
//! tests) into the person's private runtime folder, then the local voice model (LFM3-Audio through
//! `kalango-agente transcrever`) turns it into text. The recording is deleted right after.

use serde_json::Value;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub struct Recording {
    child: Child,
    path: PathBuf,
    pub started: Instant,
}

extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
    fn signal(sig: i32, handler: usize) -> usize;
    fn prctl(option: i32, arg2: u64, arg3: u64, arg4: u64, arg5: u64) -> i32;
}

pub(crate) const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;
const SIG_DFL: usize = 0;
const PR_SET_PDEATHSIG: i32 = 1;

/// A recorder child: it stops on SIGINT (it closes the WAV properly), though the shell may have been started in the
/// background with SIGINT ignored, which is inherited — so it gets the default back; and it ends with the shell (a
/// crash or a session that ends must not leave the microphone recording). The parent-death signal follows the thread
/// that spawned it: spawn from a thread that outlives the recording (the main loop, or the captions' own loop).
pub(crate) fn as_recorder(cmd: &mut Command) {
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            signal(SIGINT, SIG_DFL);
            prctl(PR_SET_PDEATHSIG, SIGTERM as u64, 0, 0, 0);
            Ok(())
        });
    }
}

/// Sends a signal to a child (the recorder's SIGINT).
pub(crate) fn signal_child(pid: u32, sig: i32) {
    unsafe {
        kill(pid as i32, sig);
    }
}

/// The person's private runtime folder for the engine (0700), where recordings live for a moment.
pub fn private_dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let d = base.join("kalango");
    let _ = std::fs::create_dir_all(&d);
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700));
    d
}

pub fn agent_binary() -> PathBuf {
    if let Some(p) = std::env::var_os("KALANGO_AGENTE") {
        return p.into();
    }
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("kalango-agente"))).filter(|p| p.is_file())
        .unwrap_or_else(|| "kalango-agente".into())
}

/// Recordings left behind by a shell that was killed mid-dictation (its process no longer exists) are deleted.
fn remove_stale_recordings(dir: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_prefix("voz-").and_then(|n| n.strip_suffix(".wav")) else { continue };
        let pid = stem.rsplit('-').next().and_then(|p| p.parse::<u32>().ok());
        if let Some(pid) = pid {
            if pid != std::process::id() && !std::path::Path::new(&format!("/proc/{pid}")).exists() {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// The file of a new recording: unique in this process (a counter before the pid, which stays the last part of the
/// name for `remove_stale_recordings`). A window closed while listening discards its recording in the background
/// for up to two seconds; reopened and listening again at once, the new one must not be the file that discard deletes.
fn recording_path(dir: &std::path::Path, tag: &str) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    dir.join(format!("voz-{tag}-{n}-{}.wav", std::process::id()))
}

pub fn start(tag: &str) -> Result<Recording, String> {
    remove_stale_recordings(&private_dir());
    let path = recording_path(&private_dir(), tag);
    let mut cmd = match std::env::var_os("KALANGO_GRAVADOR") {
        Some(p) => Command::new(p),
        None => {
            let mut c = Command::new("pw-record");
            c.args(["--rate", "16000", "--channels", "1", "--format", "s16"]);
            c
        }
    };
    as_recorder(&mut cmd);
    let child = cmd.arg(&path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    Ok(Recording { child, path, started: Instant::now() })
}

impl Recording {
    #[cfg(test)]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// How loud the last tenth of a second was (0 – 1), from the end of the file being written (16 kHz, 16 bits).
    pub fn level(&self) -> f32 {
        use std::io::{Read, Seek, SeekFrom};
        let Ok(mut f) = std::fs::File::open(&self.path) else { return 0.0 };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if len < 44 + 3200 {
            return 0.0;
        }
        let mut buf = vec![0u8; 3200];
        if f.seek(SeekFrom::Start((len - 3200) & !1)).is_err() || f.read_exact(&mut buf).is_err() {
            return 0.0;
        }
        let n = buf.len() / 2;
        let sum: f64 = buf.chunks_exact(2).map(|b| f64::from(i16::from_le_bytes([b[0], b[1]])).powi(2)).sum();
        let rms = (sum / n as f64).sqrt() / 32768.0;
        // speech sits around -30 dBFS: shown on a scale that makes it fill the bars
        ((20.0 * rms.max(1e-6).log10() + 60.0) / 50.0).clamp(0.0, 1.0) as f32
    }
}

/// Stops the recording and throws it away (nothing is transcribed).
pub fn discard(mut rec: Recording) {
    unsafe {
        kill(rec.child.id() as i32, SIGINT);
    }
    let t0 = Instant::now();
    while rec.child.try_wait().ok().flatten().is_none() && t0.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = rec.child.kill();
    let _ = rec.child.wait();
    let _ = std::fs::remove_file(&rec.path);
}

/// Stops the recording and transcribes it (blocking: call it off the UI thread). The file is deleted.
pub fn finish(rec: Recording) -> Result<String, String> {
    finish_keeping(rec, None).map(|(t, _)| t)
}

/// As `finish`, the recording kept in `keep` (a folder) instead of thrown away: (the transcript, the file).
pub fn finish_keeping(mut rec: Recording, keep: Option<PathBuf>) -> Result<(String, Option<PathBuf>), String> {
    unsafe {
        kill(rec.child.id() as i32, SIGINT);
    }
    // up to 3 s to finish the file; then it is stopped for good
    let t0 = Instant::now();
    while rec.child.try_wait().ok().flatten().is_none() {
        if t0.elapsed() > Duration::from_secs(3) {
            let _ = rec.child.kill();
            let _ = rec.child.wait();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = Command::new(agent_binary()).args(["--json", "transcrever"]).arg(&rec.path).output();
    // (kept: moved where the conversation keeps its files; otherwise gone)
    let kept = keep.and_then(|d| {
        let _ = std::fs::create_dir_all(&d);
        let hora = gtk::glib::DateTime::now_local().ok().and_then(|t| t.format("%H%M%S").ok()).map(|t| t.to_string()).unwrap_or_default();
        let dest = d.join(format!("mensagem-de-voz-{hora}.wav"));
        std::fs::rename(&rec.path, &dest).or_else(|_| std::fs::copy(&rec.path, &dest).map(|_| ())).ok().map(|_| dest)
    });
    let _ = std::fs::remove_file(&rec.path);
    let o = out.map_err(|e| e.to_string())?;
    let line = String::from_utf8_lossy(&o.stdout).lines().last().unwrap_or("").to_string();
    let v: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
    match v["tipo"].as_str() {
        Some("transcricao") => Ok((v["texto"].as_str().unwrap_or("").to_string(), kept)),
        _ => Err(v["texto"].as_str().map(str::to_string).unwrap_or_else(|| String::from_utf8_lossy(&o.stderr).trim().to_string())),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn stale_recordings_of_dead_processes_are_removed() {
        let d = std::env::temp_dir().join(format!("kalango-voz-teste-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let mine = d.join(format!("voz-conversa-{}.wav", std::process::id()));
        // PID 0x7ffffffe is far above pid_max: never a live process
        let dead = d.join("voz-comando-2147483646.wav");
        let other = d.join("notas.txt");
        for f in [&mine, &dead, &other] {
            std::fs::write(f, b"x").unwrap();
        }
        super::remove_stale_recordings(&d);
        assert!(mine.exists() && other.exists() && !dead.exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn every_recording_has_its_own_file_and_the_pid_stays_last() {
        let d = std::path::Path::new("/nada");
        let (a, b) = (super::recording_path(d, "ajuda"), super::recording_path(d, "ajuda"));
        assert_ne!(a, b, "two recordings of the same process never share a file");
        let stem = a.file_name().unwrap().to_string_lossy().trim_end_matches(".wav").to_string();
        assert_eq!(stem.rsplit('-').next().unwrap(), std::process::id().to_string(), "{stem}");
        // a leftover of a dead process, in the new name, is still cleaned up
        let dir = std::env::temp_dir().join(format!("kalango-voz-teste-nome-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dead = dir.join("voz-ajuda-3-2147483646.wav");
        std::fs::write(&dead, b"x").unwrap();
        super::remove_stale_recordings(&dir);
        assert!(!dead.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
