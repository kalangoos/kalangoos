//! "Gerar PDF" for a LaTeX document in an answer (design "Chat · diagramas, documentos e realce de sintaxe" › Documento
//! LaTeX): offered only when a TeX engine is installed (tectonic first, then pdflatex, xelatex or lualatex). The
//! document is written next to the conversation and compiled there in the background; tectonic downloads the LaTeX
//! packages a document needs the first time, so its progress is shown; the PDF opens in KalangoOS's own reader
//! (Documentos).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Where a block's conversation keeps its files (set on the conversation's root widget as "conversa_pasta").
pub type FolderFn = std::rc::Rc<dyn Fn() -> PathBuf>;

/// A TeX engine on this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Engine {
    pub path: PathBuf,
    /// tectonic (downloads its packages) or a classic *latex
    pub tectonic: bool,
}

/// The first engine found on PATH (and in ~/.local/bin, where a user installs one without root).
pub fn engine() -> Option<Engine> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    // (~/.local/bin, where a person installs one without root, even when it is not on PATH)
    if let Some(h) = std::env::var_os("HOME") {
        dirs.push(Path::new(&h).join(".local/bin"));
    }
    ["tectonic", "pdflatex", "xelatex", "lualatex"].iter().find_map(|name| {
        dirs.iter().map(|d| d.join(name)).find(|p| p.is_file()).map(|path| Engine { path, tectonic: *name == "tectonic" })
    })
}

/// The memory a compilation may take (tectonic peaks near 260 MB on a first run that downloads its packages).
pub const MEMORY_MAX: &str = "1G";
/// How long a compilation may take: a first run downloads the packages it needs (about 100 s on a home connection).
pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(240);
/// How much the drawn documents may keep in the cache (the oldest used go first).
pub const CACHE_CAP: u64 = 200 * 1024 * 1024;
/// Compilations at a time (more wait their turn).
pub const PARALLEL: usize = 2;

/// How much tectonic's package cache may hold (a document can make it fetch packages: the cache does not grow past
/// this). The packages a few common documents need (article, geometry, hyperref, enumitem, xcolor) take about 45 MB.
pub const BUNDLE_CAP: u64 = 512 * 1024 * 1024;
/// How much a compilation may write into its own folder (it is in memory), and the largest file.
pub const WORK_CAP: u64 = 128 * 1024 * 1024;
pub const FILE_CAP: u64 = 64 * 1024 * 1024;
/// Fetch rounds for a document's missing packages (a package needs others: each round fetches what the last asked).
// No fetching by name: the names come from the compile's output, which the document controls (a document could
// forge "File `x' not found" lines and make the networked fetcher fill the shared cache, or pass bits to a later
// document through what is cached). Only the trusted first compilation (a fixed document) fills the cache; a
// document asking for a package that isn't there fails with the package's name.
const FETCH_ROUNDS: usize = 0;
const FETCH_NAMES: usize = 40;

// ------------------------------------------------------------------------------------------------- the sandbox
//
// A document is the model's text: TeX can read any file the person can (\openin, \input) and write any (\openout),
// and "-Z continue-on-errors" goes on past what fails. So the engine runs walled in, whatever the document says:
// - Landlock: it reads the engine, the system's libraries and the few files TLS, name lookup and fonts need (not
//   /etc/passwd, not the home) and tectonic's package cache — READ ONLY: no compilation of a document can write
//   anything another one reads (a document that wrote into the cache once made every later "article" print its text);
//   it writes only a new folder for itself; no network at all (tectonic runs with --only-cached);
// - its folder is in memory and capped (64 MB a file, 128 MB in all), an empty environment (HOME and the XDG folders
//   inside it), a systemd scope with a memory ceiling, a time limit;
// - only the PDF is copied out, by the shell, into the cache of drawn documents.
// The package cache is filled by trusted steps that carry none of the document's text: a first compilation of a fixed
// document (the formats and the basic packages), then, for a document that asks for a package not there yet, a fetch
// of that file BY NAME from tectonic's bundle (`tectonic -X bundle cat NAME`: the content is the bundle's, the name is
// checked), up to [`BUNDLE_CAP`]. These two run walled in too: they may write the package cache and reach port 443.
// The walls go up in a small helper (`kalango-shell --latex-sandbox …`, before GTK starts, one thread) that then
// becomes the engine; on a kernel without Landlock it refuses (the document stays as code).

/// What a walled-in run is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// a document's compilation: package cache read-only, no network
    Compile,
    /// the trusted first compilation (a fixed document): writes the package cache, port 443
    Seed,
    /// the trusted fetch of one bundle file by name: writes the package cache, port 443
    Fetch(String),
}

/// One run's walls: the engine, whether it is tectonic, its own folder and tectonic's package cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sandbox {
    pub engine: PathBuf,
    pub tectonic: bool,
    pub work: PathBuf,
    pub bundles: PathBuf,
}

/// The fixed document of the first, trusted compilation: what most answers use, fetched once.
pub const SEED: &str = "\\documentclass{article}\\usepackage[utf8]{inputenc}\\usepackage[margin=1in]{geometry}\\usepackage{hyperref}\\usepackage{enumitem}\\usepackage{xcolor}\\usepackage{amsmath}\\usepackage{graphicx}\\begin{document}\\title{T}\\author{A}\\maketitle\\section*{S}\\begin{enumerate}\\item \\textbf{b} \\textit{i} $x^2$\\end{enumerate}\\begin{verbatim}v\\end{verbatim}{\\tiny a}{\\scriptsize a}{\\footnotesize a}{\\small a}{\\large a}{\\Large a}{\\LARGE a}{\\huge a}{\\Huge a}\\texttt{t}\\textsc{s}\\end{document}";

/// The engine's arguments in its folder. A document: from the cache only (no network), and like LaTeX run by hand it
/// goes on past the errors it can (a stray "\\" after \title stops nothing; the error is said beside the page).
pub fn engine_args(tectonic: bool, mode: &Mode) -> Vec<String> {
    match (tectonic, mode) {
        (true, Mode::Fetch(name)) => ["-X", "bundle", "cat", name].map(String::from).to_vec(),
        (true, Mode::Seed) => ["-Z", "continue-on-errors", "--outdir", ".", "--chatter", "default", "documento.tex"].map(String::from).to_vec(),
        (true, Mode::Compile) => ["--only-cached", "-Z", "continue-on-errors", "--outdir", ".", "--chatter", "default", "documento.tex"].map(String::from).to_vec(),
        (false, _) => ["-interaction=nonstopmode", "-output-directory=.", "documento.tex"].map(String::from).to_vec(),
    }
}

/// The engine's whole environment: nothing of the shell's, its home inside its folder.
pub fn sandbox_env(s: &Sandbox) -> Vec<(String, String)> {
    let w = s.work.display().to_string();
    vec![
        ("HOME".into(), w.clone()),
        ("XDG_CACHE_HOME".into(), format!("{w}/.cache")),
        ("XDG_CONFIG_HOME".into(), format!("{w}/.config")),
        ("XDG_DATA_HOME".into(), format!("{w}/.local/share")),
        ("TMPDIR".into(), w.clone()),
        ("TECTONIC_CACHE_DIR".into(), s.bundles.display().to_string()),
        ("TEXMFVAR".into(), format!("{w}/.texmf-var")),
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("LANG".into(), "C.UTF-8".into()),
    ]
}

/// What the engine may read (and run): the system's programs and libraries, the files TLS, name lookup and fonts need,
/// a TeX installation's trees, the engine itself — and, for a document, the package cache (read-only).
pub fn readable(s: &Sandbox, mode: &Mode) -> Vec<PathBuf> {
    let mut r: Vec<PathBuf> = ["/usr", "/lib", "/lib64", "/bin", "/etc/ssl", "/etc/ca-certificates", "/etc/resolv.conf", "/run/systemd/resolve",
        "/etc/hosts", "/etc/nsswitch.conf", "/etc/gai.conf", "/etc/host.conf", "/etc/localtime", "/etc/ld.so.cache", "/etc/fonts", "/var/cache/fontconfig",
        "/etc/texmf", "/var/lib/texmf", "/dev/urandom", "/sys/devices/system/cpu"]
        .iter().map(PathBuf::from).collect();
    r.push(s.engine.clone());
    if *mode == Mode::Compile {
        r.push(s.bundles.clone());
    }
    r
}

/// What it may write: its folder, and — only for the trusted steps — the package cache.
pub fn writable(s: &Sandbox, mode: &Mode) -> Vec<PathBuf> {
    let mut w = vec![s.work.clone(), PathBuf::from("/dev/null")];
    if *mode != Mode::Compile {
        w.push(s.bundles.clone());
    }
    w
}

/// The Landlock rules for a run, made (not yet applied). TCP: none for a document; port 443 for the trusted steps.
pub fn ruleset(s: &Sandbox, mode: &Mode) -> Result<landlock::RulesetCreated, String> {
    use landlock::{Access, AccessFs, AccessNet, NetPort, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr, ABI};
    let abi = ABI::V4;
    let mut rs = Ruleset::default().handle_access(AccessFs::from_all(abi)).map_err(|e| e.to_string())?
        .handle_access(AccessNet::from_all(abi)).map_err(|e| e.to_string())?
        .create().map_err(|e| e.to_string())?;
    let rules = readable(s, mode).into_iter().map(|p| (p, AccessFs::from_read(abi)))
        .chain(writable(s, mode).into_iter().map(|p| (p, AccessFs::from_all(abi))));
    for (p, access) in rules {
        let Ok(fd) = PathFd::new(&p) else { continue };
        // (a rule on a file takes a file's rights only)
        let a = if p.is_dir() { access } else { access & AccessFs::from_file(abi) };
        rs = rs.add_rule(PathBeneath::new(fd, a)).map_err(|e| e.to_string())?;
    }
    if *mode != Mode::Compile {
        rs = rs.add_rule(NetPort::new(443, AccessNet::ConnectTcp)).map_err(|e| e.to_string())?;
    }
    Ok(rs)
}

/// Puts the walls up on this process (no_new_privs too); an error when the kernel cannot keep the files apart.
pub fn restrict(rs: landlock::RulesetCreated) -> Result<(), String> {
    use landlock::RulesetStatus;
    let st = rs.restrict_self().map_err(|e| e.to_string())?;
    match st.ruleset {
        RulesetStatus::NotEnforced => Err("Landlock is not available in this kernel".into()),
        // (an older kernel may keep the files apart but not filter TCP: that is still the protection that matters)
        _ => Ok(()),
    }
}

/// The size limit of any file the run writes (RLIMIT_FSIZE: past it, the write fails).
fn limit_file_size() {
    let r = libc::rlimit { rlim_cur: FILE_CAP, rlim_max: FILE_CAP };
    unsafe {
        libc::setrlimit(libc::RLIMIT_FSIZE, &r);
    }
}

/// `kalango-shell --latex-sandbox ENGINE TECTONIC(0|1) WORK BUNDLES MODE [NAME]`: puts the walls up, then becomes the
/// engine.
pub fn sandbox_main(args: &[String]) -> ! {
    let fail = |m: String| -> ! {
        eprintln!("error: sandbox: {m}");
        std::process::exit(97)
    };
    let (s, mode) = match args {
        [engine, tectonic, work, bundles, mode, rest @ ..] => {
            let s = Sandbox { engine: engine.into(), tectonic: tectonic == "1", work: work.into(), bundles: bundles.into() };
            let mode = match (mode.as_str(), rest) {
                ("compile", []) => Mode::Compile,
                ("seed", []) => Mode::Seed,
                ("fetch", [name]) if valid_name(name) => Mode::Fetch(name.clone()),
                _ => fail("bad mode".into()),
            };
            (s, mode)
        }
        _ => fail("bad arguments".into()),
    };
    if mode != Mode::Compile {
        let _ = std::fs::create_dir_all(&s.bundles);
    }
    limit_file_size();
    if let Err(e) = ruleset(&s, &mode).and_then(restrict) {
        fail(e)
    }
    let err = std::os::unix::process::CommandExt::exec(Command::new(&s.engine).args(engine_args(s.tectonic, &mode)).env_clear().envs(sandbox_env(&s)).current_dir(&s.work));
    fail(format!("{}: {err}", s.engine.display()))
}

/// The arguments the helper takes for a run.
fn helper_args(s: &Sandbox, mode: &Mode) -> Vec<std::ffi::OsString> {
    let mut a: Vec<std::ffi::OsString> = vec!["--latex-sandbox".into(), s.engine.clone().into(), (if s.tectonic { "1" } else { "0" }).into(), s.work.clone().into(), s.bundles.clone().into()];
    match mode {
        Mode::Compile => a.push("compile".into()),
        Mode::Seed => a.push("seed".into()),
        Mode::Fetch(n) => {
            a.push("fetch".into());
            a.push(n.into());
        }
    }
    a
}

/// The command for a run: in a memory-capped systemd scope when systemd-run is there, the helper that puts the walls
/// up and becomes the engine; in a process group of its own (stopped whole).
fn command(s: &Sandbox, mode: &Mode, scoped: bool) -> Command {
    #[cfg(test)]
    if let Some(f) = TEST_COMMAND.lock().ok().and_then(|g| g.clone()) {
        return f(s, mode);
    }
    let helper = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("kalango-shell"));
    let systemd_run = ["/usr/bin/systemd-run", "/bin/systemd-run"].iter().map(Path::new).find(|p| p.is_file()).filter(|_| scoped);
    let mut c = match systemd_run {
        Some(sr) => {
            let mut c = Command::new(sr);
            c.args(["--user", "--scope", "--quiet", "--collect", "-p"]).arg(format!("MemoryMax={MEMORY_MAX}")).args(["-p", "MemorySwapMax=0", "--"]).arg(helper);
            c
        }
        None => Command::new(helper),
    };
    c.args(helper_args(s, mode));
    c.current_dir(&s.work).stdin(Stdio::null()).stdout(if matches!(mode, Mode::Fetch(_)) { Stdio::null() } else { Stdio::piped() }).stderr(Stdio::piped());
    std::os::unix::process::CommandExt::process_group(&mut c, 0);
    c
}

/// Tests put a stand-in engine here (the helper is the shell's binary, not the test's).
#[cfg(test)]
type TestCommand = std::sync::Arc<dyn Fn(&Sandbox, &Mode) -> Command + Send + Sync>;
#[cfg(test)]
pub static TEST_COMMAND: std::sync::Mutex<Option<TestCommand>> = std::sync::Mutex::new(None);

/// A compiled document: its PDF and the error LaTeX went past, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compiled {
    pub pdf: PathBuf,
    pub warning: Option<String>,
}

/// tectonic's packages, kept between compilations (written only by the trusted steps).
pub fn bundles_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(p) = TEST_BUNDLES.lock().ok().and_then(|g| g.clone()) {
        return p;
    }
    gtk::glib::user_cache_dir().join("kalango").join("tectonic")
}

/// Tests keep the package cache out of the home.
#[cfg(test)]
pub static TEST_BUNDLES: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);
/// One test at a time uses the stand-ins above.
#[cfg(test)]
pub static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A new folder for one run, outside the home (the runtime folder, in memory).
fn work_dir() -> std::io::Result<PathBuf> {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let base = gtk::glib::user_runtime_dir().join("kalango-latex");
    let d = base.join(format!("{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

/// A bundle file's name as a document may ask for it: letters, digits and . _ + -, not starting with a dot, short.
pub fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 80 && !n.starts_with('.') && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
}

/// The files a compilation said it could not find (`File `x.sty' not found`, a font's metric file), valid names only,
/// each once.
pub fn missing_files(output: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in output.lines() {
        let mut found = Vec::new();
        if let Some(i) = l.find("File `") {
            if let Some(j) = l[i + 6..].find("' not found") {
                found.push(l[i + 6..i + 6 + j].to_string());
            }
        }
        // "! Font \T1/cmr/m/n/10=ecrm1000 at 10.0pt not loadable: Metric (TFM) file not found." and (an OpenType font)
        // "Font TU/lmr/m/n/10.95=[lmroman10-regular]:mapping=tex-text; at 10.95pt not loadable: Metric (TFM) file or
        // installed font not found"
        if l.contains("not loadable: Metric (TFM) file") {
            if let Some(eq) = l.find('=') {
                let rest = &l[eq + 1..];
                let name = match rest.strip_prefix('[') {
                    Some(r) => {
                        let n: String = r.chars().take_while(|c| *c != ']').collect();
                        if n.contains('.') { n } else { format!("{n}.otf") }
                    }
                    None => format!("{}.tfm", rest.chars().take_while(|c| !c.is_whitespace() && !matches!(c, ':' | ';')).collect::<String>()),
                };
                found.push(name);
            }
        }
        for f in found {
            if valid_name(&f) && !out.contains(&f) {
                out.push(f);
            }
        }
    }
    out
}

/// The bytes under a folder.
pub fn dir_size(p: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else { return 0 };
    rd.flatten().map(|e| match e.metadata() {
        Ok(m) if m.is_dir() => dir_size(&e.path()),
        Ok(m) => m.len(),
        Err(_) => 0,
    }).sum()
}

/// Whether the package cache was filled by the trusted first compilation.
fn seeded(bundles: &Path) -> bool {
    bundles.join("kalango-pronto").is_file()
}

/// One walled-in run in a new folder (`src` as documento.tex when given); the run's output, the PDF if it made one,
/// and why it failed if it did.
fn walled_run(e: &Engine, src: Option<&str>, mode: &Mode, step: std::sync::Arc<dyn Fn(Progress) + Send + Sync>) -> (Result<(PathBuf, String), Failure>, PathBuf) {
    let work = match work_dir() {
        Ok(w) => w,
        Err(err) => return (Err(Failure::Document(err.to_string())), PathBuf::new()),
    };
    let s = Sandbox { engine: e.path.clone(), tectonic: e.tectonic, work: work.clone(), bundles: bundles_dir() };
    if let Some(src) = src {
        if let Err(err) = std::fs::write(work.join("documento.tex"), src) {
            return (Err(Failure::Document(err.to_string())), work);
        }
    }
    let timeout = if matches!(mode, Mode::Fetch(_)) { std::time::Duration::from_secs(60) } else { TIMEOUT };
    (run(command(&s, mode, true), &work, &work.join("documento.pdf"), timeout, step), work)
}

/// Compiles `src` walled in and puts the PDF in `out_dir` as `out_name`; `step` hears the progress. The first time,
/// the trusted first compilation fills the package cache; a document asking for packages not there gets them fetched
/// by name, then compiled again. Blocking: run it in a thread.
#[allow(clippy::absurd_extreme_comparisons)] // (FETCH_ROUNDS is 0 on purpose: see it)
pub fn compile(e: &Engine, src: &str, out_dir: &Path, out_name: &str, step: impl Fn(Progress) + Send + Sync + 'static) -> Result<Compiled, Failure> {
    let step: std::sync::Arc<dyn Fn(Progress) + Send + Sync> = std::sync::Arc::new(step);
    let bundles = bundles_dir();
    if e.tectonic && !seeded(&bundles) {
        step(Progress::Preparing);
        let (r, work) = walled_run(e, Some(SEED), &Mode::Seed, step.clone());
        let _ = std::fs::remove_dir_all(&work);
        match r {
            Ok(_) => {
                let _ = std::fs::write(bundles.join("kalango-pronto"), b"");
            }
            Err(f) => return Err(f),
        }
    }
    let mut asked: Vec<String> = Vec::new();
    for round in 0..=FETCH_ROUNDS {
        let (r, work) = walled_run(e, Some(src), &Mode::Compile, step.clone());
        // packages it asked for and does not have: fetch them (by name, trusted), then again
        let missing: Vec<String> = match &r {
            Ok((_, log)) | Err(Failure::Missing(log)) => missing_files(log).into_iter().filter(|n| !asked.contains(n)).take(FETCH_NAMES).collect(),
            _ => Vec::new(),
        };
        if e.tectonic && !missing.is_empty() && round < FETCH_ROUNDS {
            let _ = std::fs::remove_dir_all(&work);
            if dir_size(&bundles) > BUNDLE_CAP {
                return Err(Failure::CacheFull);
            }
            for n in &missing {
                step(Progress::Downloading(n.clone()));
                let (_, w) = walled_run(e, None, &Mode::Fetch(n.clone()), step.clone());
                let _ = std::fs::remove_dir_all(&w);
                asked.push(n.clone());
            }
            continue;
        }
        let out = r.and_then(|(pdf, log)| {
            std::fs::create_dir_all(out_dir).map_err(|err| Failure::Document(err.to_string()))?;
            let to = out_dir.join(out_name);
            std::fs::copy(&pdf, &to).map_err(|err| Failure::Document(err.to_string()))?;
            // (went past an error: the page is there, the error is said)
            let warning = match failure(&log) {
                Failure::Document(l) if !l.is_empty() => Some(l),
                _ => None,
            };
            Ok(Compiled { pdf: to, warning })
        }).map_err(|f| match f {
            Failure::Missing(log) => failure(&log),
            f => f,
        });
        let _ = std::fs::remove_dir_all(&work);
        return out;
    }
    Err(Failure::Document(String::new()))
}

/// Runs `cmd` (a run in `work` that writes `pdf`) at most `timeout`, its folder at most [`WORK_CAP`], reading its
/// output for the progress. Ok: (the PDF, the output); no PDF: why (the output, when it may be missing packages).
fn run(mut cmd: Command, work: &Path, pdf: &Path, timeout: std::time::Duration, step: std::sync::Arc<dyn Fn(Progress) + Send + Sync>) -> Result<(PathBuf, String), Failure> {
    use std::io::BufRead;
    let mut child = cmd.spawn().map_err(|err| Failure::Document(err.to_string()))?;
    let read = |pipe: Box<dyn std::io::Read + Send>, step: std::sync::Arc<dyn Fn(Progress) + Send + Sync>| {
        std::thread::spawn(move || {
            let mut all = String::new();
            for l in std::io::BufReader::new(pipe).lines().map_while(Result::ok) {
                if let Some(p) = progress(&l) {
                    step(p);
                }
                // (the output kept is bounded too)
                if all.len() < 1 << 20 {
                    all.push_str(&l);
                    all.push('\n');
                }
            }
            all
        })
    };
    let out = child.stdout.take().map(|o| read(Box::new(o), step.clone()));
    let err = child.stderr.take().map(|o| read(Box::new(o), step.clone()));
    let started = std::time::Instant::now();
    let mut checked = std::time::Instant::now();
    let mut stopped: Option<Failure> = None;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                let over_time = started.elapsed() > timeout;
                let over_size = checked.elapsed() > std::time::Duration::from_millis(200) && {
                    checked = std::time::Instant::now();
                    dir_size(work) > WORK_CAP
                };
                if over_time || over_size {
                    stopped = Some(if over_time { Failure::TimedOut } else { Failure::TooBig });
                    // (the whole group: everything it started stops too)
                    unsafe {
                        libc::kill(-(child.id() as i32), libc::SIGKILL);
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => break None,
        }
    };
    let mut all = String::new();
    for h in [err, out].into_iter().flatten() {
        if let Ok(t) = h.join() {
            all.push_str(&t);
        }
    }
    if let Some(f) = stopped {
        return Err(f);
    }
    if status.is_none() {
        return Err(Failure::TimedOut);
    }
    if let Some(m) = all.lines().find_map(|l| l.strip_prefix("error: sandbox: ")) {
        return Err(Failure::Sandbox(m.to_string()));
    }
    let made = pdf.is_file() && std::fs::metadata(pdf).is_ok_and(|m| m.len() > 0);
    if !made {
        return Err(if missing_files(&all).is_empty() { failure(&all) } else { Failure::Missing(all) });
    }
    Ok((pdf.to_path_buf(), all))
}

// ------------------------------------------------------------------------------------------------- the cache

/// The drawn documents' cache: one folder a text (by its hash), outside anything a compilation can write.
pub fn cache_root() -> PathBuf {
    #[cfg(test)]
    if let Some(p) = TEST_CACHE.lock().ok().and_then(|g| g.clone()) {
        return p;
    }
    gtk::glib::user_cache_dir().join("kalango").join("latex")
}

#[cfg(test)]
pub static TEST_CACHE: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// A text's key in the cache (with the engine: another engine draws it again).
pub fn cache_key(src: &str, e: &Engine) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut h);
    e.path.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// What the cache holds for a key: the PDF (and the error LaTeX went past), or why it failed, or nothing yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cached {
    Pdf(Compiled),
    Failed(String),
    Nothing,
}

pub fn cached(dir: &Path) -> Cached {
    let pdf = dir.join("documento.pdf");
    if pdf.is_file() {
        // (used now: the last to go when the cache is pruned)
        let _ = std::fs::write(dir.join("usado"), b"");
        let warning = std::fs::read_to_string(dir.join("aviso.txt")).ok().filter(|w| !w.is_empty());
        return Cached::Pdf(Compiled { pdf, warning });
    }
    match std::fs::read_to_string(dir.join("falha.txt")) {
        Ok(f) => Cached::Failed(f),
        Err(_) => Cached::Nothing,
    }
}

/// Keeps a compilation's result in its folder (the PDF is already there; the warning, or why it failed).
pub fn remember(dir: &Path, r: &Result<Compiled, Failure>, failure_text: &str) {
    let _ = std::fs::create_dir_all(dir);
    match r {
        Ok(c) => {
            let _ = std::fs::write(dir.join("aviso.txt"), c.warning.clone().unwrap_or_default());
            let _ = std::fs::remove_file(dir.join("falha.txt"));
        }
        // (no connection: worth trying again next time; the document's own error, too long, too big are kept, with
        // "Tentar de novo")
        Err(Failure::Offline) => {}
        Err(_) => {
            let _ = std::fs::write(dir.join("falha.txt"), failure_text);
        }
    }
}

/// The folders to delete so the cache fits `cap`: the least recently used first. `entries`: (folder, bytes, last use).
pub fn to_prune(entries: &[(PathBuf, u64, std::time::SystemTime)], cap: u64) -> Vec<PathBuf> {
    let mut e: Vec<&(PathBuf, u64, std::time::SystemTime)> = entries.iter().collect();
    e.sort_by_key(|x| x.2);
    let mut total: u64 = entries.iter().map(|x| x.1).sum();
    let mut out = Vec::new();
    for (p, size, _) in e {
        if total <= cap {
            break;
        }
        total -= size;
        out.push(p.clone());
    }
    out
}

/// Prunes the cache to [`CACHE_CAP`].
pub fn prune() {
    let Ok(rd) = std::fs::read_dir(cache_root()) else { return };
    let entries: Vec<(PathBuf, u64, std::time::SystemTime)> = rd.flatten().filter(|d| d.path().is_dir()).map(|d| {
        let files: Vec<std::fs::Metadata> = std::fs::read_dir(d.path()).into_iter().flatten().flatten().filter_map(|f| f.metadata().ok()).collect();
        let size = files.iter().map(|m| m.len()).sum();
        let used = files.iter().filter_map(|m| m.modified().ok()).max().unwrap_or(std::time::UNIX_EPOCH);
        (d.path(), size, used)
    }).collect();
    for p in to_prune(&entries, CACHE_CAP) {
        let _ = std::fs::remove_dir_all(p);
    }
}

// ------------------------------------------------------------------------------------------------- the queue

/// The compilations waiting and running: at most `max` at a time, one per key (a text asked for twice waits for the
/// first run).
#[derive(Debug, Default)]
pub struct Queue {
    max: usize,
    running: Vec<String>,
    waiting: std::collections::VecDeque<String>,
}

/// What to do with a request.
#[derive(Debug, PartialEq, Eq)]
pub enum Enqueued {
    /// start it now
    Start,
    /// it waits its turn
    Waiting,
    /// the same text is already running or waiting: hear its result
    Joined,
}

impl Queue {
    pub fn new(max: usize) -> Queue {
        Queue { max: max.max(1), ..Default::default() }
    }

    pub fn request(&mut self, key: &str) -> Enqueued {
        if self.running.iter().any(|k| k == key) || self.waiting.iter().any(|k| k == key) {
            return Enqueued::Joined;
        }
        if self.running.len() < self.max {
            self.running.push(key.to_string());
            Enqueued::Start
        } else {
            self.waiting.push_back(key.to_string());
            Enqueued::Waiting
        }
    }

    /// `key` finished; the next one to start, if any.
    pub fn finished(&mut self, key: &str) -> Option<String> {
        self.running.retain(|k| k != key);
        let next = self.waiting.pop_front()?;
        self.running.push(next.clone());
        Some(next)
    }
}

/// What a line of the engine's output says it is doing (for the progress), if anything worth showing.
pub fn progress(line: &str) -> Option<Progress> {
    let l = line.trim();
    if l.starts_with("note: downloading") || l.contains("downloading index") || l.starts_with("note: connecting") {
        Some(Progress::Downloading(l.rsplit(' ').next().unwrap_or("").trim_matches(['"', '\'', '`']).to_string()))
    } else if l.starts_with("note: Running TeX") || l.starts_with("This is pdfTeX") || l.starts_with("This is XeTeX") || l.starts_with("This is LuaHBTeX") {
        Some(Progress::Typesetting)
    } else if l.starts_with("note: Writing") || l.starts_with("Output written on") {
        Some(Progress::Writing)
    } else {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    /// the first compilation, filling the package cache (only the first time)
    Preparing,
    /// a package being downloaded (its name)
    Downloading(String),
    Typesetting,
    Writing,
}

/// Why it failed, for the person: no network (the first run downloads packages) or the document's own error.
pub fn failure(output: &str) -> Failure {
    let low = output.to_lowercase();
    let offline = ["failed to lookup address", "dns error", "network is unreachable", "connection refused", "error sending request", "could not resolve host", "temporary failure in name resolution", "no route to host", "timed out", "failed to connect"];
    if offline.iter().any(|o| low.contains(o)) {
        return Failure::Offline;
    }
    // the first error line (tectonic: "error: …", LaTeX: "! …")
    let line = output.lines().map(str::trim).find(|l| (l.starts_with("error:") && !l.contains("halted on potentially-recoverable")) || l.starts_with('!'))
        .or_else(|| output.lines().map(str::trim).find(|l| l.starts_with("error:")))
        .unwrap_or("").trim_start_matches("error:").trim_start_matches('!').trim().to_string();
    Failure::Document(line)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Offline,
    /// it ran past [`TIMEOUT`]
    TimedOut,
    /// its folder grew past [`WORK_CAP`]
    TooBig,
    /// the package cache is full ([`BUNDLE_CAP`])
    CacheFull,
    /// the walls could not go up (a kernel without Landlock): not compiled
    Sandbox(String),
    /// no PDF, and packages missing (its output: what to fetch)
    Missing(String),
    Document(String),
}

/// A file name for the document in `dir` that is not taken (documento.tex, documento-2.tex…).
pub fn free_name(dir: &Path) -> PathBuf {
    (1..).map(|n| dir.join(if n == 1 { "documento.tex".to_string() } else { format!("documento-{n}.tex") }))
        .find(|p| !p.exists() && !p.with_extension("pdf").exists()).unwrap_or_else(|| dir.join("documento.tex"))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn progress_lines() {
        assert_eq!(progress("note: downloading article.cls"), Some(Progress::Downloading("article.cls".into())));
        assert_eq!(progress("note: Running TeX ..."), Some(Progress::Typesetting));
        assert_eq!(progress("note: Writing `documento.pdf` (12.34 KiB)"), Some(Progress::Writing));
        assert_eq!(progress("This is pdfTeX, Version 3.141592653"), Some(Progress::Typesetting));
        assert_eq!(progress("(./documento.aux)"), None);
    }

    #[test]
    fn why_it_failed() {
        let off = "note: connecting to https://relay.fullyjustified.net/default_bundle_v33.tar\nerror: failed to open bundle\ncaused by: error sending request for url (https://relay.fullyjustified.net/…): dns error: failed to lookup address information";
        assert_eq!(failure(off), Failure::Offline);
        let doc = "error: forum.tex:7: Undefined control sequence\nerror: halted on potentially-recoverable error as specified";
        assert_eq!(failure(doc), Failure::Document("forum.tex:7: Undefined control sequence".into()));
        assert_eq!(failure("! LaTeX Error: File `xyz.sty' not found."), Failure::Document("LaTeX Error: File `xyz.sty' not found.".into()));
    }

    #[test]
    fn missing_packages_and_names() {
        let out = "error: d.tex:1: ! LaTeX Error: File `xcolor.sty' not found.\nerror: d.tex:1: ! LaTeX Error: File `tikz.sty' not found.\nerror: d.tex:1: ! LaTeX Error: File `xcolor.sty' not found.\n! Font \\T1/cmr/m/n/10=ecrm1000 at 10.0pt not loadable: Metric (TFM) file not found.\n! LaTeX Error: File `../../.bashrc' not found.\n! LaTeX Error: File `/etc/passwd' not found.";
        assert_eq!(missing_files(out), ["xcolor.sty", "tikz.sty", "ecrm1000.tfm"]);
        // (the font a 10.95 pt document asked for, live: the text came out blank without it)
        let otf = "warning: size11.clo:54: Font TU/lmr/m/n/10.95=[lmroman10-regular]:mapping=tex-text; at 10.95pt not loadable: Metric (TFM) file or installed font not found";
        assert_eq!(missing_files(otf), ["lmroman10-regular.otf"]);
        assert!(valid_name("pgfcore.code.tex") && valid_name("t1cmr.fd"));
        assert!(!valid_name("../x") && !valid_name("/etc/passwd") && !valid_name(".hidden") && !valid_name("a b") && !valid_name(&"a".repeat(81)));
    }

    /// A stand-in engine (a shell script, run by sh — a script just written can be "busy" while another test forks):
    /// what these test is how a run is read and stopped, not TeX.
    fn fake(dir: &Path, body: &str) -> Command {
        let f = dir.join("engine.sh");
        std::fs::write(&f, format!("{body}\n")).unwrap();
        let mut c = Command::new("/bin/sh");
        c.arg(f).current_dir(dir).stdout(Stdio::piped()).stderr(Stdio::piped());
        std::os::unix::process::CommandExt::process_group(&mut c, 0);
        c
    }

    pub fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kalango-latex-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn quiet() -> std::sync::Arc<dyn Fn(Progress) + Send + Sync> {
        std::sync::Arc::new(|_| {})
    }

    #[test]
    fn runs_are_read_stopped_and_capped() {
        let dir = temp("run");
        // past an error: the page and the output with the error
        let c = fake(&dir, "echo \"forum.tex:11: LaTeX Error: There's no line here to end.\" >&2\necho \"! LaTeX Error: There's no line here to end.\"\nprintf '%%PDF-1.4' > documento.pdf");
        let (pdf, out) = run(c, &dir, &dir.join("documento.pdf"), TIMEOUT, quiet()).unwrap();
        assert_eq!(pdf, dir.join("documento.pdf"));
        assert_eq!(failure(&out), Failure::Document("LaTeX Error: There's no line here to end.".into()));
        let _ = std::fs::remove_file(dir.join("documento.pdf"));
        // no page and packages missing: what to fetch
        let c = fake(&dir, "echo \"! LaTeX Error: File \\`tikz.sty' not found.\" >&2");
        assert!(matches!(run(c, &dir, &dir.join("documento.pdf"), TIMEOUT, quiet()), Err(Failure::Missing(o)) if missing_files(&o) == ["tikz.sty"]));
        // too long: the whole group stopped, at once
        let c = fake(&dir, "sleep 30 & sleep 30");
        let t = std::time::Instant::now();
        assert_eq!(run(c, &dir, &dir.join("documento.pdf"), std::time::Duration::from_millis(400), quiet()), Err(Failure::TimedOut));
        assert!(t.elapsed() < std::time::Duration::from_secs(5), "{:?}", t.elapsed());
        // too big a folder: stopped
        let c = fake(&dir, "while :; do head -c 8388608 /dev/zero >> grande.bin; done");
        assert_eq!(run(c, &dir, &dir.join("documento.pdf"), TIMEOUT, quiet()), Err(Failure::TooBig));
        let _ = std::fs::remove_file(dir.join("grande.bin"));
        // the helper's own refusal (no Landlock) is said as such
        let c = fake(&dir, "echo 'error: sandbox: Landlock is not available in this kernel' >&2; exit 97");
        assert_eq!(run(c, &dir, &dir.join("documento.pdf"), TIMEOUT, quiet()), Err(Failure::Sandbox("Landlock is not available in this kernel".into())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_walls_by_mode() {
        let s = Sandbox { engine: "/x/tectonic".into(), tectonic: true, work: "/run/user/1/kalango-latex/1-0".into(), bundles: "/h/.cache/kalango/tectonic".into() };
        let c = command(&s, &Mode::Compile, true);
        let args: Vec<String> = c.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        if Path::new("/usr/bin/systemd-run").is_file() && TEST_COMMAND.lock().map(|g| g.is_none()).unwrap_or(false) {
            // a scope with a memory ceiling, the helper inside it
            assert_eq!(c.get_program(), "/usr/bin/systemd-run");
            assert!(args.contains(&format!("MemoryMax={MEMORY_MAX}")) && args.contains(&"MemorySwapMax=0".to_string()));
        }
        let h: Vec<String> = helper_args(&s, &Mode::Fetch("tikz.sty".into())).iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(h, ["--latex-sandbox", "/x/tectonic", "1", "/run/user/1/kalango-latex/1-0", "/h/.cache/kalango/tectonic", "fetch", "tikz.sty"]);
        // a document: from the cache only, reading it, not writing it, no network rule
        assert!(engine_args(true, &Mode::Compile).contains(&"--only-cached".to_string()));
        assert!(readable(&s, &Mode::Compile).contains(&s.bundles) && !writable(&s, &Mode::Compile).contains(&s.bundles));
        // the trusted steps: they write the cache (and fetch by name)
        assert!(writable(&s, &Mode::Seed).contains(&s.bundles) && writable(&s, &Mode::Fetch("a.sty".into())).contains(&s.bundles));
        assert_eq!(engine_args(true, &Mode::Fetch("a.sty".into())), ["-X", "bundle", "cat", "a.sty"]);
        // a document never decides what is fetched into the shared cache
        assert_eq!(FETCH_ROUNDS, 0);
        // an environment of its own: nothing of the shell's; reading: not /etc/passwd, not the home
        let env = sandbox_env(&s);
        assert!(env.iter().any(|(k, v)| k == "HOME" && v == "/run/user/1/kalango-latex/1-0"));
        assert!(!env.iter().any(|(k, _)| k == "DISPLAY" || k == "WAYLAND_DISPLAY" || k == "DBUS_SESSION_BUS_ADDRESS"));
        assert!(!readable(&s, &Mode::Seed).iter().any(|p| p == Path::new("/etc") || p == Path::new("/etc/passwd") || p.starts_with("/home")));
    }

    #[test]
    fn cache_keys_failures_and_pruning() {
        let t = Engine { path: "/x/tectonic".into(), tectonic: true };
        assert_eq!(cache_key("a", &t), cache_key("a", &t));
        assert_ne!(cache_key("a", &t), cache_key("b", &t));
        let dir = temp("cache");
        assert_eq!(cached(&dir), Cached::Nothing);
        // no connection: tried again next time; the document's own error and a timeout are kept ("Tentar de novo")
        remember(&dir, &Err(Failure::Offline), "sem rede");
        assert_eq!(cached(&dir), Cached::Nothing);
        remember(&dir, &Err(Failure::TimedOut), "demorou");
        assert_eq!(cached(&dir), Cached::Failed("demorou".into()));
        remember(&dir, &Err(Failure::Document("x".into())), "O LaTeX não conseguiu gerar o PDF: x");
        assert_eq!(cached(&dir), Cached::Failed("O LaTeX não conseguiu gerar o PDF: x".into()));
        std::fs::write(dir.join("documento.pdf"), b"%PDF").unwrap();
        remember(&dir, &Ok(Compiled { pdf: dir.join("documento.pdf"), warning: Some("w".into()) }), "");
        assert_eq!(cached(&dir), Cached::Pdf(Compiled { pdf: dir.join("documento.pdf"), warning: Some("w".into()) }));
        let _ = std::fs::remove_dir_all(&dir);
        // the least recently used go first, until it fits
        let at = |s: u64| std::time::UNIX_EPOCH + std::time::Duration::from_secs(s);
        let e = vec![("a".into(), 50, at(30)), ("b".into(), 80, at(10)), ("c".into(), 40, at(20))];
        assert_eq!(to_prune(&e, 200), Vec::<PathBuf>::new());
        assert_eq!(to_prune(&e, 100), vec![PathBuf::from("b")]);
        assert_eq!(to_prune(&e, 60), vec![PathBuf::from("b"), PathBuf::from("c")]);
    }

    #[test]
    fn the_queue_runs_two_at_a_time_and_one_per_text() {
        let mut q = Queue::new(2);
        assert_eq!(q.request("a"), Enqueued::Start);
        assert_eq!(q.request("a"), Enqueued::Joined);
        assert_eq!(q.request("b"), Enqueued::Start);
        assert_eq!(q.request("c"), Enqueued::Waiting);
        assert_eq!(q.request("c"), Enqueued::Joined);
        assert_eq!(q.request("d"), Enqueued::Waiting);
        assert_eq!(q.finished("a"), Some("c".into()));
        assert_eq!(q.finished("b"), Some("d".into()));
        assert_eq!(q.finished("c"), None);
        assert_eq!(q.request("a"), Enqueued::Start);
    }

    #[test]
    fn names() {
        let dir = temp("nomes");
        assert_eq!(free_name(&dir), dir.join("documento.tex"));
        std::fs::write(dir.join("documento.tex"), "x").unwrap();
        assert_eq!(free_name(&dir), dir.join("documento-2.tex"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The real engine, walled in by the same rules, environment and file limit the helper uses (put up in the child
    /// before exec: the helper is the shell's binary, not the test's) — for [`compile`]'s whole path.
    pub fn walled_test_command() -> TestCommand {
        std::sync::Arc::new(|s: &Sandbox, mode: &Mode| {
            let rs = std::sync::Mutex::new(Some(ruleset(s, mode).unwrap()));
            let mut c = Command::new(&s.engine);
            c.args(engine_args(s.tectonic, mode)).env_clear().envs(sandbox_env(s)).current_dir(&s.work)
                .stdout(if matches!(mode, Mode::Fetch(_)) { Stdio::null() } else { Stdio::piped() }).stderr(Stdio::piped());
            std::os::unix::process::CommandExt::process_group(&mut c, 0);
            unsafe {
                std::os::unix::process::CommandExt::pre_exec(&mut c, move || {
                    limit_file_size();
                    let r = rs.lock().ok().and_then(|mut g| g.take()).ok_or_else(|| std::io::Error::other("no rules"))?;
                    restrict(r).map_err(std::io::Error::other)
                });
            }
            c
        })
    }

    /// Every file of the package cache and its content's hash (what a later compilation would read).
    fn snapshot(p: &Path) -> Vec<(PathBuf, u64)> {
        use std::hash::{Hash, Hasher};
        let mut out = Vec::new();
        let mut stack = vec![p.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let path = e.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    std::fs::read(&path).unwrap_or_default().hash(&mut h);
                    out.push((path, h.finish()));
                }
            }
        }
        out.sort();
        out
    }

    fn page_text(pdf: &Path) -> String {
        let doc = poppler::Document::from_file(&gtk::prelude::FileExt::uri(&gtk::gio::File::for_path(pdf)), None).unwrap();
        (0..doc.n_pages()).filter_map(|k| doc.page(k).and_then(|p| p.text())).map(|t| t.to_string()).collect::<String>().split_whitespace().collect()
    }

    /// The real tectonic through [`compile`]'s whole path (the first, trusted compilation; fetching by name; the
    /// document read-only and offline): a hostile document cannot write outside its folder — not the package cache, not
    /// a path of its own choosing, not "..", nor fill the disk —, cannot read the system's or the home's files, and
    /// cannot change what a later document sees; a normal document still draws. Needs tectonic (skipped without it)
    /// and its packages, kept in /tmp (outside the home) between runs: the first run downloads them (about 45 MB).
    #[test]
    fn hostile_documents_stay_inside_their_walls() {
        let Some(e) = engine().filter(|e| e.tectonic) else {
            eprintln!("(sem tectonic: o teste do isolamento não roda)");
            return;
        };
        let _g = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let bundles = std::env::var_os("KALANGO_TESTE_TECTONIC_CACHE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp/kalango-teste-tectonic"));
        std::fs::create_dir_all(&bundles).unwrap();
        *TEST_BUNDLES.lock().unwrap() = Some(bundles.clone());
        *TEST_COMMAND.lock().unwrap() = Some(walled_test_command());
        let out = temp("saida");
        let outside = temp("fora");
        let secret = outside.join("segredo.tex");
        std::fs::write(&secret, "\\def\\segredo{VAZOU}").unwrap();
        // a normal document draws (and the cache is seeded, its packages fetched)
        let normal = "\\documentclass{article}\\begin{document}Olá, \\textbf{mundo}.\\end{document}";
        let r = compile(&e, normal, &out, "normal.pdf", |_| {}).unwrap();
        assert!(page_text(&r.pdf).contains("mundo"));
        let before = snapshot(&bundles);
        // writes: into the package cache over article.cls's file, a path of its own, "..", a huge file
        let cached_cls = before.iter().map(|(p, _)| p.clone()).find(|p| p.ends_with("article.cls")).expect("article.cls in the cache");
        let escape_abs = outside.join("escapou.txt");
        let writes = format!(
            "\\documentclass{{article}}\\begin{{document}}\n\
             \\newwrite\\p\\immediate\\openout\\p={cls}\\immediate\\write\\p{{\\noexpand\\typeout{{POISONED}}\\noexpand\\AtBeginDocument{{POISONED-BY-EARLIER-DOC}}}}\\immediate\\closeout\\p\n\
             \\newwrite\\n\\immediate\\openout\\n={bundles}/novo.sty\\immediate\\write\\n{{X}}\\immediate\\closeout\\n\n\
             \\newwrite\\w\\immediate\\openout\\w={abs}\\immediate\\write\\w{{X}}\\immediate\\closeout\\w\n\
             \\newwrite\\v\\immediate\\openout\\v=../escapou-relativo.txt\\immediate\\write\\v{{X}}\\immediate\\closeout\\v\n\
             Ok.\\end{{document}}",
            cls = cached_cls.display(), bundles = bundles.display(), abs = escape_abs.display());
        let _ = compile(&e, &writes, &out, "w.pdf", |_| {});
        assert!(!escape_abs.exists(), "\\openout to an absolute path got out");
        assert!(!std::env::temp_dir().join("escapou-relativo.txt").exists() && !gtk::glib::user_runtime_dir().join("kalango-latex/escapou-relativo.txt").exists(), "\\openout to .. got out");
        assert_eq!(snapshot(&bundles), before, "a document changed the package cache");
        // a filler: a growing file (kept in memory by tectonic, capped by the scope's memory in the shell; on disk, the
        // folder's cap stops it — see runs_are_read_stopped_and_capped): nothing stays, the cache does not grow
        let filler = "\\documentclass{article}\\begin{document}\\newwrite\\g\\immediate\\openout\\g=grande.txt\\count255=0 \\loop\\immediate\\write\\g{XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX}\\advance\\count255 by 1 \\ifnum\\count255<600000 \\repeat\\immediate\\closeout\\g Ok.\\end{document}";
        let t = std::time::Instant::now();
        let _ = compile(&e, filler, &out, "g.pdf", |_| {});
        eprintln!("(o documento que enche o disco parou em {:?})", t.elapsed());
        assert_eq!(snapshot(&bundles), before, "a document grew the package cache");
        let left: Vec<_> = std::fs::read_dir(gtk::glib::user_runtime_dir().join("kalango-latex")).into_iter().flatten().flatten().collect();
        assert!(left.is_empty(), "a compilation's folder stayed: {left:?}");
        // reads: /etc/passwd and a file outside, by \openin and \input
        let reads = format!(
            "\\documentclass{{article}}\\begin{{document}}\n\
             \\newread\\r\\openin\\r=/etc/passwd\\ifeof\\r\\def\\lido{{SEMACESSO}}\\else\\read\\r to\\lido\\fi\n\
             \\IfFileExists{{{secret}}}{{\\input{{{secret}}}}}{{\\def\\segredo{{SEMACESSO}}}}\n\
             passwd=\\detokenize\\expandafter{{\\lido}};casa=\\segredo.\\end{{document}}",
            secret = secret.display());
        let got = page_text(&compile(&e, &reads, &out, "r.pdf", |_| {}).unwrap().pdf);
        assert!(got.contains("passwd=SEMACESSO") && !got.contains("root:"), "\\openin /etc/passwd was read: {got}");
        assert!(got.contains("casa=SEMACESSO") && !got.contains("VAZOU"), "\\input of a file outside got in: {got}");
        // and a later document sees what it would have seen
        let later = compile(&e, normal, &out, "depois.pdf", |_| {}).unwrap();
        let t = page_text(&later.pdf);
        assert!(t.contains("mundo") && !t.contains("POISONED"), "{t}");
        *TEST_COMMAND.lock().unwrap() = None;
        *TEST_BUNDLES.lock().unwrap() = None;
        for d in [out, outside] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}
