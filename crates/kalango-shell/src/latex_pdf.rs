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

/// The command that turns `tex` into a PDF in `dir`.
pub fn command(e: &Engine, tex: &Path, dir: &Path) -> Command {
    let mut c = Command::new(&e.path);
    if e.tectonic {
        // (-Z continue-on-errors is not used: a broken document says so)
        c.arg("--outdir").arg(dir).arg("--chatter").arg("default").arg(tex);
    } else {
        c.arg("-interaction=nonstopmode").arg("-halt-on-error").arg(format!("-output-directory={}", dir.display())).arg(tex);
    }
    c.current_dir(dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    c
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
    Document(String),
}

/// A file name for the document in `dir` that is not taken (documento.tex, documento-2.tex…).
pub fn free_name(dir: &Path) -> PathBuf {
    (1..).map(|n| dir.join(if n == 1 { "documento.tex".to_string() } else { format!("documento-{n}.tex") }))
        .find(|p| !p.exists() && !p.with_extension("pdf").exists()).unwrap_or_else(|| dir.join("documento.tex"))
}

#[cfg(test)]
mod tests {
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
    fn names_and_commands() {
        let dir = std::env::temp_dir().join(format!("kalango-latex-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(free_name(&dir), dir.join("documento.tex"));
        std::fs::write(dir.join("documento.tex"), "x").unwrap();
        assert_eq!(free_name(&dir), dir.join("documento-2.tex"));
        let e = Engine { path: "/usr/bin/pdflatex".into(), tectonic: false };
        let c = command(&e, &dir.join("documento.tex"), &dir);
        let args: Vec<String> = c.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.contains(&"-halt-on-error".to_string()));
        let t = Engine { path: "/x/tectonic".into(), tectonic: true };
        assert!(command(&t, &dir.join("a.tex"), &dir).get_args().any(|a| a == "--outdir"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
