//! Pages that only show their content with scripts (a search engine's results, sites built in the browser) are opened
//! by Moli, a headless browser engine for agents (github.com/lexmount/moli), when it is installed.
//!
//! The browser runs page code, so it is held like an agent's task:
//!   - every request it makes (the page, its scripts' requests, websockets) goes through a proxy here that refuses local
//!     and private addresses — the same rule as `web.ler`, checked on the address it connects to;
//!   - Landlock: it reads the system (not the person's home), writes only in a folder of its own that is removed after,
//!     and may only connect to the proxy's port; a cgroup limits memory, processes and time;
//!   - it gets no environment of kalangod's (tokens, keys) and runs as the person, never as root.

use super::tr;
use super::web::forbidden;
use crate::limits::{self, Landlock, Limits, Pedido};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

static NEXT: AtomicU64 = AtomicU64::new(1);

/// (After the page’s requests settle — a page built by scripts fetches its content after loading.) The page’s own code: where it ended, its title, and its HTML without scripts, styles and pictures
/// (450 KB at most: the task's output has a limit).
const EXPORT: &str = r#"(() => { const d = document.documentElement.cloneNode(true); d.querySelectorAll("script,style,svg,template,link,meta,iframe,object,embed").forEach(e => e.remove()); return JSON.stringify({url: location.href, titulo: document.title, html: d.outerHTML.slice(0, 450000)}); })()"#;

/// Where Moli is: `KALANGOD_MOLI` (empty or "nao" turns it off), then the places in `moli_places`, then the PATH.
pub fn moli() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("KALANGOD_MOLI") {
        let p = PathBuf::from(v);
        return (p.as_os_str() != "nao" && p.is_file()).then_some(p);
    }
    let exe = std::env::current_exe().ok();
    moli_places(exe.as_deref()).into_iter().find(|p| p.is_file())
        .or_else(|| std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|d| d.join("moli")).find(|p| p.is_file())))
}

/// The cargo profiles a dev build may come from (the dev session builds with "sessao", the image with "release").
const PROFILES: [&str; 3] = ["release", "sessao", "debug"];

/// Where to look for Moli, in order, given kalangod's own path: next to kalangod; when kalangod is a dev build
/// (`<repo>/target/<profile>/kalangod`), the other profiles' folders (poc/setup-deps.sh links target/release/moli)
/// and the dev dependencies' copy (`<repo>/poc/.deps/moli/moli`); the installed system's (imagem/construir.sh).
pub(crate) fn moli_places(exe: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = exe.and_then(Path::parent) {
        out.push(dir.join("moli"));
        if let Some(target) = dir.parent().filter(|t| t.file_name().is_some_and(|n| n == "target")) {
            out.extend(PROFILES.iter().map(|p| target.join(p).join("moli")).filter(|p| p.parent() != Some(dir)));
            if let Some(repo) = target.parent() {
                out.push(repo.join("poc/.deps/moli/moli"));
            }
        }
    }
    out.push(PathBuf::from("/usr/lib/kalango/moli"));
    out
}

/// The error when Moli is nowhere (shown under the canvas and the browser tab).
pub(crate) fn not_installed() -> String {
    tr("The agents' browser (Moli) is not installed on this computer.").to_string()
}

// ------------------------------------------------------------------------------------------------- the proxy

/// Which local or private addresses a page may reach: none (the rule), all (`web_local`, tests with a local page
/// server), or one origin (the session test's own pages, `KALANGOD_BUSCA_URL`).
#[derive(Clone, Debug, PartialEq)]
pub enum Local {
    No,
    All,
    Only(String, u16),
}

impl Local {
    pub(crate) fn allows(&self, host: &str, port: u16) -> bool {
        match self {
            Local::No => false,
            Local::All => true,
            Local::Only(h, p) => h.eq_ignore_ascii_case(host) && *p == port,
        }
    }
}

/// A proxy on 127.0.0.1 for one page: CONNECT (https, websockets) and absolute-form http requests, each to an address
/// that is checked (and then connected to, so a second lookup cannot point elsewhere). It stops when dropped.
pub struct Proxy {
    pub port: u16,
    stop: Arc<AtomicBool>,
}

impl Proxy {
    pub fn start(local: Local) -> std::io::Result<Proxy> {
        let l = TcpListener::bind("127.0.0.1:0")?;
        let port = l.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        std::thread::spawn(move || {
            for conn in l.incoming() {
                if s.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(c) = conn {
                    let local = local.clone();
                    std::thread::spawn(move || handle(c, &local));
                }
            }
        });
        Ok(Proxy { port, stop })
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // (wakes the listener up so it sees the stop)
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

/// The address to connect to, when the host may be reached.
pub fn allowed_addr(host: &str, port: u16, local: &Local) -> Result<SocketAddr, String> {
    let host = host.trim_matches(|c| c == '[' || c == ']');
    let local_ok = local.allows(host, port);
    let addrs: Vec<SocketAddr> = (host, port).to_socket_addrs().map_err(|e| format!("{host}: {e}"))?.collect();
    if addrs.is_empty() {
        return Err(format!("{host}: no address"));
    }
    if !local_ok && addrs.iter().any(|a| forbidden(a.ip())) {
        return Err(format!("{host}: local or private address"));
    }
    Ok(addrs[0])
}

fn split_host_port(s: &str, default: u16) -> Option<(String, u16)> {
    if let Some(rest) = s.strip_prefix('[') {
        let (h, p) = rest.split_once(']')?;
        let port = p.strip_prefix(':').map(|p| p.parse().ok()).unwrap_or(Some(default))?;
        return Some((h.to_string(), port));
    }
    match s.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => Some((h.to_string(), p.parse().ok()?)),
        _ => Some((s.to_string(), default)),
    }
}

fn refuse(c: &mut TcpStream, why: &str) {
    let _ = c.write_all(format!("HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{why}", why.len()).as_bytes());
}

fn handle(mut c: TcpStream, local: &Local) {
    let _ = c.set_read_timeout(Some(Duration::from_secs(30)));
    // the request's head (16 KiB at most)
    let mut head = Vec::new();
    let mut chunk = [0u8; 4096];
    let end = loop {
        let n = match c.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        head.extend_from_slice(&chunk[..n]);
        if let Some(p) = head.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
        if head.len() > 16 * 1024 {
            return refuse(&mut c, "request head too long");
        }
    };
    let (head_bytes, rest) = head.split_at(end);
    let text = String::from_utf8_lossy(head_bytes).to_string();
    let mut first = text.lines().next().unwrap_or("").split_whitespace();
    let (method, target, version) = (first.next().unwrap_or(""), first.next().unwrap_or(""), first.next().unwrap_or("HTTP/1.1"));
    let up = if method.eq_ignore_ascii_case("CONNECT") {
        let Some((host, port)) = split_host_port(target, 443) else { return refuse(&mut c, "bad target") };
        let addr = match allowed_addr(&host, port, local) {
            Ok(a) => a,
            Err(e) => return refuse(&mut c, &e),
        };
        let Ok(mut up) = TcpStream::connect_timeout(&addr, Duration::from_secs(10)) else { return refuse(&mut c, "could not connect") };
        if c.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").is_err() || up.write_all(rest).is_err() {
            return;
        }
        up
    } else {
        // http://host[:port]/path → "METHOD /path VERSION" to that host, one request per connection
        let Ok(u) = url::Url::parse(target) else { return refuse(&mut c, "absolute http address expected") };
        if u.scheme() != "http" {
            return refuse(&mut c, "only http through here");
        }
        let host = u.host_str().unwrap_or("").to_string();
        let addr = match allowed_addr(&host, u.port().unwrap_or(80), local) {
            Ok(a) => a,
            Err(e) => return refuse(&mut c, &e),
        };
        let Ok(mut up) = TcpStream::connect_timeout(&addr, Duration::from_secs(10)) else { return refuse(&mut c, "could not connect") };
        let path = &u[url::Position::BeforePath..url::Position::AfterQuery];
        let mut out = format!("{method} {} {version}\r\n", if path.is_empty() { "/" } else { path });
        for line in text.lines().skip(1) {
            if line.is_empty() {
                break;
            }
            let l = line.to_ascii_lowercase();
            if l.starts_with("proxy-") || l.starts_with("connection:") || l.starts_with("keep-alive:") {
                continue;
            }
            out.push_str(line);
            out += "\r\n";
        }
        out += "Connection: close\r\n\r\n";
        if up.write_all(out.as_bytes()).is_err() || up.write_all(rest).is_err() {
            return;
        }
        up
    };
    pipe(c, up);
}

fn pipe(a: TcpStream, b: TcpStream) {
    let _ = a.set_read_timeout(Some(Duration::from_secs(60)));
    let _ = b.set_read_timeout(Some(Duration::from_secs(60)));
    let (Ok(mut a2), Ok(mut b2)) = (a.try_clone(), b.try_clone()) else { return };
    let t = std::thread::spawn(move || {
        let mut a = a;
        let mut b = b;
        let _ = std::io::copy(&mut a, &mut b);
        let _ = b.shutdown(Shutdown::Write);
    });
    let _ = std::io::copy(&mut b2, &mut a2);
    let _ = a2.shutdown(Shutdown::Write);
    let _ = t.join();
}

// ------------------------------------------------------------------------------------------------- the browser

/// What the browser saw: where the page ended, its title, and its HTML after the scripts ran.
pub struct Rendered {
    pub url: String,
    pub title: String,
    pub html: String,
}

/// Opens the page in Moli (held as described above). `uid`/`gid`: the person's.
pub fn render(url: &str, uid: u32, gid: u32, local: Local) -> Result<Rendered, String> {
    render_with(url, uid, gid, local, false)
}

/// As `render`; `strict`: a page that answers 4xx/5xx is an error, not its error page (a site that refuses even the
/// browser — a login wall — must not be read as if it were the page).
pub fn render_with(url: &str, uid: u32, gid: u32, local: Local, strict: bool) -> Result<Rendered, String> {
    // (the real file: Landlock gives the folder it is in, not a link's)
    let moli = moli().ok_or_else(not_installed)?;
    let moli = std::fs::canonicalize(&moli).unwrap_or(moli);
    let proxy = Proxy::start(local).map_err(|e| format!("proxy: {e}"))?;
    let dir = std::env::temp_dir().join(format!("kalango-navegador-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    if nix::unistd::geteuid().is_root() {
        let _ = nix::unistd::chown(&dir, Some(nix::unistd::Uid::from_raw(uid)), Some(nix::unistd::Gid::from_raw(gid)));
    }
    let mut args: Vec<String> = ["fetch", "--eval", EXPORT, "--wait", "networkidle", "--timeout-ms", "15000", "--http-proxy", &format!("http://127.0.0.1:{}", proxy.port),
        "--http-max-response-size", "8388608", "--disable-subframes"].iter().map(|s| s.to_string()).collect();
    if strict {
        args.push("--fail".into());
    }
    args.push(url.to_string());
    let env: BTreeMap<String, String> = [("PATH", "/usr/bin:/bin"), ("LANG", "C.UTF-8")].iter().map(|(k, v)| (k.to_string(), v.to_string()))
        .chain([("HOME", &dir), ("TMPDIR", &dir), ("XDG_CACHE_HOME", &dir)].iter().map(|(k, v)| (k.to_string(), v.display().to_string())))
        .collect();
    let mut leitura: Vec<PathBuf> = ["/usr", "/lib", "/lib64", "/bin", "/etc", "/proc", "/sys", "/dev"].iter().map(PathBuf::from).collect();
    if let Some(d) = moli.parent() {
        leitura.push(d.to_path_buf());
    }
    let landlock = Landlock { leitura, listar: vec![], escrita: vec![dir.clone(), PathBuf::from("/dev/null"), PathBuf::from("/dev/shm")], portas: Some(vec![proxy.port]), fixas: vec![], sockets: vec![] };
    let lim = Limits { memoria_mb: 1024, cpu_pct: 200, processos: 128, tempo_s: 30 };
    let programa = moli.display().to_string();
    let r = limits::run_task(Pedido { programa: &programa, argumentos: &args, uid, gid, limites: lim, landlock: Some(landlock),
        // (no other program's local socket: the session bus would let a page's exploit start programs outside the browser's
        // isolation; the network is the proxy's one port, by Landlock)
        seccomp: Some(limits::Seccomp { sem_rede: false, sem_unix: true }),
        ambiente: Some(env), pasta: Some(dir.clone()), dry_run: false, tmp_privado: false, extra: vec![] });
    drop(proxy);
    let _ = std::fs::remove_dir_all(&dir);
    let r = r?;
    if r["estourou_tempo"].as_bool() == Some(true) {
        return Err("the browser took too long".into());
    }
    if r["codigo"].as_i64() != Some(0) {
        let err = r["erros"].as_str().unwrap_or("").lines().last().unwrap_or("").to_string();
        return Err(format!("the browser failed: {err}"));
    }
    let out = r["saida"].as_str().unwrap_or("");
    let v: Value = serde_json::from_str(out.trim()).map_err(|e| format!("the browser's answer: {e}"))?;
    Ok(Rendered {
        url: v["url"].as_str().unwrap_or(url).to_string(),
        title: v["titulo"].as_str().unwrap_or("").to_string(),
        html: v["html"].as_str().unwrap_or("").to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moli_is_looked_for_beside_kalangod_in_the_other_profiles_and_the_system() {
        // the dev session's build (target/sessao): its own folder first, then release's link, then the deps' copy
        let got = moli_places(Some(Path::new("/r/target/sessao/kalangod")));
        let want: Vec<PathBuf> = ["/r/target/sessao/moli", "/r/target/release/moli", "/r/target/debug/moli", "/r/poc/.deps/moli/moli", "/usr/lib/kalango/moli"]
            .iter().map(PathBuf::from).collect();
        assert_eq!(got, want);
        // installed: beside it, then the system's; no guessing at dev folders
        assert_eq!(moli_places(Some(Path::new("/usr/bin/kalangod"))), vec![PathBuf::from("/usr/bin/moli"), PathBuf::from("/usr/lib/kalango/moli")]);
        assert_eq!(moli_places(None), vec![PathBuf::from("/usr/lib/kalango/moli")]);
    }

    #[test]
    fn a_missing_moli_is_said_in_portuguese() {
        let pt = kalango_i18n::catalog(Some("pt_BR"));
        assert_eq!(pt.get("The agents' browser (Moli) is not installed on this computer.").map(String::as_str), Some("O navegador dos agentes (Moli) não está instalado neste computador."));
    }

    fn ask(port: u16, req: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut out = Vec::new();
        let mut buf = [0u8; 1024];
        while let Ok(n) = s.read(&mut buf) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
            if out.windows(4).any(|w| w == b"\r\n\r\n") && !req.starts_with("GET") {
                break;
            }
        }
        String::from_utf8_lossy(&out).to_string()
    }

    #[test]
    fn the_proxy_refuses_local_addresses_and_carries_the_rest() {
        // a local page (allowed only when local addresses are allowed, as in the session test)
        let page = TcpListener::bind("127.0.0.1:0").unwrap();
        let pport = page.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for c in page.incoming().flatten() {
                let mut c = c;
                let mut buf = [0u8; 2048];
                let n = c.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let line = req.lines().next().unwrap_or("").to_string();
                let body = format!("got {line}");
                let _ = c.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes());
            }
        });
        let strict = Proxy::start(Local::No).unwrap();
        for req in [format!("CONNECT 127.0.0.1:{pport} HTTP/1.1\r\nHost: x\r\n\r\n"), format!("GET http://127.0.0.1:{pport}/a HTTP/1.1\r\nHost: x\r\n\r\n"),
            "CONNECT 192.168.1.1:443 HTTP/1.1\r\n\r\n".into(), "CONNECT [::1]:443 HTTP/1.1\r\n\r\n".into(), "GET http://localhost/ HTTP/1.1\r\n\r\n".into()] {
            let r = ask(strict.port, &req);
            assert!(r.starts_with("HTTP/1.1 403"), "{req:?} → {r}");
        }
        let open = Proxy::start(Local::All).unwrap();
        let r = ask(open.port, &format!("GET http://127.0.0.1:{pport}/caminho?x=1 HTTP/1.1\r\nHost: 127.0.0.1\r\nProxy-Connection: keep-alive\r\n\r\n"));
        assert!(r.contains("got GET /caminho?x=1 HTTP/1.1"), "{r}");
        let r = ask(open.port, &format!("CONNECT 127.0.0.1:{pport} HTTP/1.1\r\n\r\n"));
        assert!(r.starts_with("HTTP/1.1 200 Connection Established"), "{r}");
        // one origin: that one, not another name for the same machine
        let one = Proxy::start(Local::Only("127.0.0.1".into(), pport)).unwrap();
        assert!(ask(one.port, &format!("GET http://127.0.0.1:{pport}/a HTTP/1.1\r\n\r\n")).contains("got GET /a"));
        assert!(ask(one.port, &format!("GET http://localhost:{pport}/b HTTP/1.1\r\n\r\n")).starts_with("HTTP/1.1 403"));
        assert_eq!(split_host_port("[::1]:8443", 443), Some(("::1".into(), 8443)));
        assert_eq!(split_host_port("exemplo.com", 443), Some(("exemplo.com".into(), 443)));
    }
}
