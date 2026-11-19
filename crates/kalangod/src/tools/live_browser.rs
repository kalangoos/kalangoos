//! The agents' visible browser (design "Chat · navegador do agente"): the pages an agent reads are shown in the side
//! bar as Moli draws them, the part it read outlined, and the person may take over — their clicks, scrolling and
//! typing go to the same page, without reloading it.
//!
//! One Moli per person, kept alive between calls (`moli serve --remote-debugging-pipe`: the Chrome DevTools protocol
//! over two pipes, file descriptors 3 and 4 — no port anyone else could reach). It is held exactly like the one that
//! reads pages for `web.ler` (see navegador.rs): its requests go through a proxy that refuses local and private
//! addresses, Landlock leaves it the system to read and a folder of its own to write, a cgroup limits memory,
//! processes and time, it runs as the person with none of kalangod's environment, and it starts with no profile
//! (none of the person's logins). It closes after 15 minutes unused, and lives one hour at most.
//!
//! The shell drives it through `kalango/navegador` (server.rs); each action answers with a picture of the page.

use super::navegador::{moli, Local, Proxy};
use crate::limits::{self, Landlock, Limits, Pedido};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const IDLE: Duration = Duration::from_secs(15 * 60);
const LIFETIME_S: u64 = 3600;
static NEXT: AtomicU64 = AtomicU64::new(1);

/// The page's size in the browser (the side bar shows it scaled).
const WIDTH: u32 = 1100;
const HEIGHT: u32 = 760;

struct Session {
    child: Child,
    to: std::fs::File,
    from: Receiver<Value>,
    next: u64,
    target: String,
    started: Instant,
    used: Instant,
    width: u32,
    height: u32,
    /// the address last asked for (an error page shows it, not the engine's own address)
    requested: String,
    /// the pages of this session, kept here: Moli's own history misses a navigation made by a click on a link (suite
    /// run 53: "Voltar" stayed off after following a link)
    back_stack: Vec<String>,
    fwd_stack: Vec<String>,
    dir: PathBuf,
    _proxy: Proxy,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.send("Browser.close", json!({}), false);
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn sessions() -> &'static Mutex<HashMap<u32, Arc<Mutex<Option<Session>>>>> {
    static S: OnceLock<Mutex<HashMap<u32, Arc<Mutex<Option<Session>>>>>> = OnceLock::new();
    S.get_or_init(|| {
        // the ones left unused close by themselves
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_secs(60));
            let all: Vec<_> = sessions().lock().map(|m| m.values().cloned().collect()).unwrap_or_default();
            for s in all {
                if let Ok(mut g) = s.try_lock() {
                    if g.as_ref().is_some_and(|s| s.used.elapsed() > IDLE || s.started.elapsed().as_secs() >= LIFETIME_S) {
                        *g = None;
                    }
                }
            }
        });
        Mutex::new(HashMap::new())
    })
}

/// A pipe whose two ends close on exec (the child's end is given to it as 3 or 4 explicitly).
fn pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

impl Session {
    fn start(uid: u32, gid: u32, local: Local) -> Result<Session, String> {
        // (the real file: Landlock gives the folder it is in, not a link's)
        let moli = moli().ok_or("Moli is not installed")?;
        let moli = std::fs::canonicalize(&moli).unwrap_or(moli);
        let proxy = Proxy::start(local).map_err(|e| format!("proxy: {e}"))?;
        let dir = std::env::temp_dir().join(format!("kalango-navegador-vivo-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        if nix::unistd::geteuid().is_root() {
            let _ = nix::unistd::chown(&dir, Some(nix::unistd::Uid::from_raw(uid)), Some(nix::unistd::Gid::from_raw(gid)));
        }
        let args: Vec<String> = ["serve", "--remote-debugging-pipe", "--layout", "--http-proxy", &format!("http://127.0.0.1:{}", proxy.port),
            "--http-max-response-size", "8388608", "--disable-subframes"].iter().map(|s| s.to_string()).collect();
        let env: BTreeMap<String, String> = [("PATH", "/usr/bin:/bin"), ("LANG", "C.UTF-8")].iter().map(|(k, v)| (k.to_string(), v.to_string()))
            .chain([("HOME", &dir), ("TMPDIR", &dir), ("XDG_CACHE_HOME", &dir)].iter().map(|(k, v)| (k.to_string(), v.display().to_string())))
            .collect();
        let mut leitura: Vec<PathBuf> = ["/usr", "/lib", "/lib64", "/bin", "/etc", "/proc", "/sys", "/dev"].iter().map(PathBuf::from).collect();
        if let Some(d) = moli.parent() {
            leitura.push(d.to_path_buf());
        }
        let landlock = Landlock { leitura, escrita: vec![dir.clone(), PathBuf::from("/dev/null"), PathBuf::from("/dev/shm")], portas: Some(vec![proxy.port]) };
        let lim = Limits { memoria_mb: 1024, cpu_pct: 200, processos: 128, tempo_s: LIFETIME_S };
        let programa = moli.display().to_string();
        let prep = limits::prepare_command(Pedido { programa: &programa, argumentos: &args, uid, gid, limites: lim, landlock: Some(landlock), seccomp: None,
            ambiente: Some(env), pasta: Some(dir.clone()), dry_run: false }, "kalango-navegador")?;
        let mut cmd = prep.cmd;
        // fd 3: what Moli reads (our writes); fd 4: what it writes (our reads)
        let (child_reads, we_write) = pipe().map_err(|e| e.to_string())?;
        let (we_read, child_writes) = pipe().map_err(|e| e.to_string())?;
        let (cr, cw) = (child_reads.as_raw_fd(), child_writes.as_raw_fd());
        let err = std::fs::File::create(dir.join("moli.err")).map_err(|e| e.to_string())?;
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(err).env("LC_ALL", "C");
        unsafe {
            cmd.pre_exec(move || {
                // (out of the way first: one of them may already be 3 or 4)
                let a = libc::fcntl(cr, libc::F_DUPFD_CLOEXEC, 100);
                let b = libc::fcntl(cw, libc::F_DUPFD_CLOEXEC, 100);
                if a < 0 || b < 0 || libc::dup2(a, 3) < 0 || libc::dup2(b, 4) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd.spawn().map_err(|e| format!("start the browser: {e}"))?;
        drop(child_reads);
        drop(child_writes);
        // the messages, each ended by a NUL
        let (tx, rx) = channel();
        let mut r = std::fs::File::from(we_read);
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 65536];
            while let Ok(n) = r.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                while let Some(i) = buf.iter().position(|&b| b == 0) {
                    let msg: Vec<u8> = buf.drain(..=i).collect();
                    if let Ok(v) = serde_json::from_slice::<Value>(&msg[..msg.len() - 1]) {
                        if tx.send(v).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        let mut s = Session { child, to: std::fs::File::from(we_write), from: rx, next: 1, target: String::new(), started: Instant::now(), used: Instant::now(),
            width: WIDTH, height: HEIGHT, requested: String::new(), back_stack: Vec::new(), fwd_stack: Vec::new(), dir, _proxy: proxy };
        let t = s.call("Target.createTarget", json!({"url": "about:blank"}), false, 20)?;
        let tid = t["targetId"].as_str().ok_or("the browser did not open a page")?.to_string();
        let a = s.call("Target.attachToTarget", json!({"targetId": tid, "flatten": true}), false, 20)?;
        s.target = a["sessionId"].as_str().ok_or("the browser did not attach to the page")?.to_string();
        s.call("Page.enable", json!({}), true, 10)?;
        s.size(WIDTH, HEIGHT)?;
        Ok(s)
    }

    fn send(&mut self, method: &str, params: Value, page: bool) -> Result<u64, String> {
        let id = self.next;
        self.next += 1;
        let mut m = json!({"id": id, "method": method, "params": params});
        if page {
            m["sessionId"] = json!(self.target);
        }
        let mut bytes = serde_json::to_vec(&m).map_err(|e| e.to_string())?;
        bytes.push(0);
        self.to.write_all(&bytes).map_err(|e| format!("the browser closed: {e}"))?;
        Ok(id)
    }

    fn call(&mut self, method: &str, params: Value, page: bool, timeout_s: u64) -> Result<Value, String> {
        let id = self.send(method, params, page)?;
        let deadline = Instant::now() + Duration::from_secs(timeout_s);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.from.recv_timeout(left) {
                Ok(v) if v["id"].as_u64() == Some(id) => {
                    return match v.get("error") {
                        Some(e) => Err(format!("{method}: {}", e["message"].as_str().unwrap_or("error"))),
                        None => Ok(v["result"].clone()),
                    };
                }
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => return Err(format!("{method}: the browser took too long")),
                Err(RecvTimeoutError::Disconnected) => return Err("the browser closed".into()),
            }
        }
    }

    fn eval(&mut self, expr: &str) -> Result<Value, String> {
        let r = self.call("Runtime.evaluate", json!({"expression": expr, "returnByValue": true, "awaitPromise": true}), true, 20)?;
        if let Some(e) = r.get("exceptionDetails") {
            return Err(format!("script: {}", e["text"].as_str().unwrap_or("error")));
        }
        Ok(r["result"]["value"].clone())
    }

    fn size(&mut self, w: u32, h: u32) -> Result<(), String> {
        self.call("Emulation.setDeviceMetricsOverride", json!({"width": w, "height": h, "deviceScaleFactor": 1, "mobile": false}), true, 10)?;
        self.width = w;
        self.height = h;
        Ok(())
    }

    /// Until the page settles: loaded, and a moment for what its scripts fetch (15 s at most).
    fn settle(&mut self) {
        std::thread::sleep(Duration::from_millis(250));
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if self.eval("document.readyState").ok().and_then(|v| v.as_str().map(str::to_string)).as_deref() == Some("complete") {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        std::thread::sleep(Duration::from_millis(600));
    }

    /// Where the page is now, and its picture.
    fn view(&mut self) -> Result<Value, String> {
        let mut info = self.eval("[location.href, document.title, scrollY]").unwrap_or(json!(["", "", 0]));
        if info[0].as_str().is_some_and(|u| u.starts_with("chrome-error:") || u == "about:blank") && !self.requested.is_empty() {
            info[0] = json!(self.requested);
        }
        // (a page reached any way — opened, a link followed, a form sent — is a step back; a new one clears forward)
        if let Some(u) = info[0].as_str().filter(|u| u.starts_with("http")) {
            if self.back_stack.last().map(String::as_str) != Some(u) {
                self.back_stack.push(u.to_string());
                if self.fwd_stack.last().map(String::as_str) != Some(u) {
                    self.fwd_stack.clear();
                } else {
                    self.fwd_stack.pop();
                }
            }
        }
        let shot = self.call("Page.captureScreenshot", json!({"format": "png"}), true, 20)?;
        Ok(json!({
            "url": info[0], "titulo": info[1], "rolagem": info[2],
            "foto": shot["data"], "largura": self.width, "altura": self.height,
            // (the first entry is the blank page the browser starts on)
            "voltar": self.back_stack.len() > 1, "avancar": !self.fwd_stack.is_empty(),
        }))
    }

    fn history(&mut self, delta: i64) -> Result<(), String> {
        let target = if delta < 0 {
            if self.back_stack.len() < 2 {
                return Ok(());
            }
            let cur = self.back_stack.pop().unwrap_or_default();
            self.fwd_stack.push(cur);
            self.back_stack.last().cloned()
        } else {
            let Some(u) = self.fwd_stack.last().cloned() else { return Ok(()) };
            Some(u)
        };
        if let Some(u) = target {
            self.requested = u.clone();
            self.call("Page.navigate", json!({"url": u}), true, 30)?;
            self.settle();
        }
        Ok(())
    }

    fn click(&mut self, x: f64, y: f64) -> Result<(), String> {
        for t in ["mouseMoved", "mousePressed", "mouseReleased"] {
            self.call("Input.dispatchMouseEvent", json!({"type": t, "x": x, "y": y, "button": "left", "clickCount": 1}), true, 10)?;
        }
        self.settle();
        Ok(())
    }

    fn key(&mut self, key: &str) -> Result<(), String> {
        let (code, vk, text) = match key {
            "Enter" => ("Enter", 13, "\r"),
            "Tab" => ("Tab", 9, ""),
            "Backspace" => ("Backspace", 8, ""),
            "Escape" => ("Escape", 27, ""),
            "ArrowDown" => ("ArrowDown", 40, ""),
            "ArrowUp" => ("ArrowUp", 38, ""),
            "PageDown" => ("PageDown", 34, ""),
            "PageUp" => ("PageUp", 33, ""),
            _ => return Err(format!("unknown key: {key}")),
        };
        let mut down = json!({"type": "keyDown", "key": key, "code": code, "windowsVirtualKeyCode": vk});
        if !text.is_empty() {
            down["text"] = json!(text);
        }
        self.call("Input.dispatchKeyEvent", down, true, 10)?;
        self.call("Input.dispatchKeyEvent", json!({"type": "keyUp", "key": key, "code": code, "windowsVirtualKeyCode": vk}), true, 10)?;
        if key == "Enter" {
            self.settle();
        }
        Ok(())
    }

    /// The part of the page that has most of these words (what the agent read): brought into view, and its box.
    fn outline(&mut self, words: &[String]) -> Result<Value, String> {
        let script = format!(r#"(() => {{
  // (a number matches only as a number — "43" is not in "438" —, with a decimal comma or point)
  const esc = w => w.replace(/[.*+?^${{}}()|[\]\\]/g, "\\$&");
  const tests = {words}.map(w => /^[0-9][0-9.,]*$/.test(w)
    ? new RegExp("(^|[^0-9.,])(" + esc(w) + "|" + esc(w.replace(/[.,]/g, m => m === "," ? "." : ",")) + ")(?![0-9])")
    : {{ test: t => t.includes(w) }});
  let best = null, score = 0;
  for (const el of document.body.querySelectorAll("p,li,h1,h2,h3,h4,h5,h6,td,th,tr,dd,dt,blockquote,pre,figcaption,a,span,div")) {{
    // (what a page says, not its references, menus or footer)
    if (el.closest("nav,footer,header,.references,.reflist,.navbox,.mw-references-wrap,[role=navigation]")) continue;
    const t = (el.innerText || el.textContent || "").toLowerCase();
    if (t.length < 8 || t.length > 700) continue;
    let n = 0;
    for (const r of tests) if (r.test(t)) n++;
    if (n === 0) continue;
    // (what is on the page: not a hidden menu or a folded box)
    const b = el.getBoundingClientRect();
    if (b.width < 8 || b.height < 8 || b.height > innerHeight * 0.45) continue;
    // (the smallest block with the most words: a sentence's paragraph, not the whole page)
    const s = n * 100 - t.length / 20;
    if (s > score) {{ score = s; best = el; }}
  }}
  if (!best) return null;
  best.scrollIntoView({{block: "center"}});
  window.__kalangoTrecho = best;
  return (best.innerText || best.textContent || "").trim().slice(0, 240);
}})()"#, words = serde_json::to_string(words).unwrap_or_else(|_| "[]".into()));
        let trecho = self.eval(&script)?;
        if trecho.is_null() {
            return Ok(Value::Null);
        }
        std::thread::sleep(Duration::from_millis(250));
        // (measured after scrolling, which moved it: Moli's getBoundingClientRect keeps the place it had before)
        // (its box against the document's own box: right whether the engine's boxes follow the scrolling or not)
        let r = self.eval("(() => { const b = window.__kalangoTrecho.getBoundingClientRect(); const d = document.documentElement.getBoundingClientRect(); return [b.left - d.left - scrollX, b.top - d.top - scrollY, b.width, b.height]; })()")?;
        Ok(json!({"x": r[0], "y": r[1], "w": r[2], "h": r[3], "trecho": trecho}))
    }
}

/// The browser's address rule: local and private addresses refused, except the session test's own pages.
fn local_rule(all_local: bool) -> Local {
    if all_local {
        return Local::All;
    }
    match super::web::test_origin() {
        Some((h, p)) => Local::Only(h, p),
        None => Local::No,
    }
}

/// `kalango/navegador`: {acao: abrir (url) | ver | clicar (x, y) | rolar (dy) | digitar (texto) | tecla (tecla) | voltar |
/// avancar | recarregar | destacar (palavras) | fechar, largura?, altura?} → the page's address, title and picture.
pub fn act(uid: u32, gid: u32, all_local: bool, p: &Value) -> Result<Value, String> {
    let acao = p["acao"].as_str().unwrap_or("ver");
    let slot = sessions().lock().map_err(|_| "browser state")?.entry(uid).or_insert_with(|| Arc::new(Mutex::new(None))).clone();
    let mut g = slot.lock().map_err(|_| "browser state")?;
    if acao == "fechar" {
        *g = None;
        return Ok(json!({"fechado": true}));
    }
    // (a browser that died or reached its hour starts again)
    if g.as_mut().is_some_and(|s| !matches!(s.child.try_wait(), Ok(None)) || s.started.elapsed().as_secs() >= LIFETIME_S) {
        *g = None;
    }
    if g.is_none() {
        if acao != "abrir" {
            return Err("the browser is closed".into());
        }
        *g = Some(Session::start(uid, gid, local_rule(all_local))?);
    }
    let s = g.as_mut().unwrap();
    s.used = Instant::now();
    let w = p["largura"].as_u64().map(|v| v.clamp(320, 2400) as u32).unwrap_or(s.width);
    let h = p["altura"].as_u64().map(|v| v.clamp(240, 2400) as u32).unwrap_or(s.height);
    if (w, h) != (s.width, s.height) {
        s.size(w, h)?;
    }
    let mut extra = json!({});
    let r = (|| -> Result<(), String> {
        match acao {
            "abrir" => {
                let url = p["url"].as_str().ok_or("no address")?;
                let u = url::Url::parse(url).map_err(|e| format!("{url}: {e}"))?;
                if !matches!(u.scheme(), "http" | "https") {
                    return Err(format!("{url}: only web pages"));
                }
                s.requested = url.to_string();
                let r = s.call("Page.navigate", json!({"url": url}), true, 30)?;
                // (a page that answers 4xx/5xx is still shown: a login wall is what the person came to see)
                if let Some(e) = r["errorText"].as_str().filter(|e| !e.is_empty() && !e.contains("HTTP_RESPONSE_CODE")) {
                    return Err(format!("{url}: {e}"));
                }
                s.settle();
            }
            "ver" => {}
            "clicar" => s.click(p["x"].as_f64().ok_or("no x")?, p["y"].as_f64().ok_or("no y")?)?,
            "rolar" => {
                let dy = p["dy"].as_f64().unwrap_or(0.0);
                s.call("Input.dispatchMouseEvent", json!({"type": "mouseWheel", "x": s.width as f64 / 2.0, "y": s.height as f64 / 2.0, "deltaX": 0, "deltaY": dy}), true, 10)?;
                std::thread::sleep(Duration::from_millis(200));
            }
            "digitar" => {
                s.call("Input.insertText", json!({"text": p["texto"].as_str().unwrap_or("")}), true, 10)?;
            }
            "tecla" => s.key(p["tecla"].as_str().unwrap_or(""))?,
            "voltar" => s.history(-1)?,
            "avancar" => s.history(1)?,
            "recarregar" => {
                s.call("Page.reload", json!({}), true, 30)?;
                s.settle();
            }
            "destacar" => {
                let words: Vec<String> = p["palavras"].as_array().map(|a| a.iter().filter_map(|w| w.as_str().map(|w| w.to_lowercase())).collect()).unwrap_or_default();
                extra = json!({"destaque": s.outline(&words)?});
            }
            other => return Err(format!("unknown action: {other}")),
        }
        Ok(())
    })();
    if let Err(e) = r {
        // a browser that stopped answering does not stay
        if e.contains("closed") || e.contains("took too long") {
            *g = None;
        }
        return Err(e);
    }
    let mut v = s.view()?;
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_childs_pipe_ends_close_on_exec() {
        let (a, b) = pipe().unwrap();
        for fd in [a.as_raw_fd(), b.as_raw_fd()] {
            assert_ne!(unsafe { libc::fcntl(fd, libc::F_GETFD) } & libc::FD_CLOEXEC, 0);
        }
    }

    #[test]
    fn only_the_tests_own_pages_are_local() {
        assert_eq!(local_rule(true), Local::All);
        if std::env::var_os("KALANGOD_BUSCA_URL").is_none() {
            assert_eq!(local_rule(false), Local::No);
        }
    }
}
