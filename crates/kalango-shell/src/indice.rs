//! A small file index for the command bar's universal search: names of the person's files, built in the
//! background (the desktop's indexer may be off), refreshed every 10 minutes. Only names, sizes and dates —
//! never the content. Heavy or hidden folders are skipped (.git, node_modules, target, caches…).

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    name_lower: String,
    pub size: u64,
    pub modified: i64,
}

#[derive(Clone, Default)]
pub struct FileIndex {
    entries: Arc<RwLock<Vec<Entry>>>,
}

const MAX_ENTRIES: usize = 250_000;
const MAX_DEPTH: usize = 6;
const SKIP: &[&str] = &["node_modules", "target", "__pycache__", "venv", ".venv", "site-packages", "build", "dist", "snap"];

fn walk(dir: &Path, depth: usize, out: &mut Vec<Entry>) {
    if depth > MAX_DEPTH || out.len() >= MAX_ENTRIES {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if out.len() >= MAX_ENTRIES {
            return;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || SKIP.contains(&name.as_str()) {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let path = e.path();
        if ft.is_dir() {
            walk(&path, depth + 1, out);
        } else if let Ok(md) = e.metadata() {
            let modified = md.modified().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
            out.push(Entry { name_lower: name.to_lowercase(), path, size: md.len(), modified });
        }
    }
}

impl FileIndex {
    /// Builds now (in a thread) and every 10 minutes.
    pub fn start() -> FileIndex {
        let idx = FileIndex::default();
        let entries = idx.entries.clone();
        std::thread::spawn(move || loop {
            let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
            let mut out = Vec::new();
            walk(&home, 0, &mut out);
            if let Ok(mut e) = entries.write() {
                *e = out;
            }
            std::thread::sleep(Duration::from_secs(600));
        });
        idx
    }

    /// Files whose name matches the query. Short connecting words are ignored ("relatório de outubro"), a word
    /// matches a part of the name that starts it or that it starts (so "out" in "relatorio-out.pdf" matches
    /// "outubro"), and at least half of the words must match. More words matched first; then the name starting
    /// with the first word, a part starting with it, containing it; then more recent.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Entry> {
        let words = significant_words(query);
        if words.is_empty() {
            return vec![];
        }
        let need = words.len().div_ceil(2);
        let Ok(entries) = self.entries.read() else { return vec![] };
        let mut hits: Vec<(usize, u8, &Entry)> = entries.iter().filter_map(|e| {
            let name = fold(&e.name_lower);
            let parts: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|p| !p.is_empty()).collect();
            let matched = words.iter().filter(|w| word_matches(w, &name, &parts)).count();
            if matched < need {
                return None;
            }
            let first = &words[0];
            let rank = if name.starts_with(first.as_str()) { 0 } else if parts.iter().any(|p| p.starts_with(first.as_str())) { 1 } else { 2 };
            Some((matched, rank, e))
        }).collect();
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(b.2.modified.cmp(&a.2.modified)));
        hits.into_iter().take(limit).map(|(_, _, e)| e.clone()).collect()
    }

    /// Text files that contain the phrase (accents and case ignored), with the line where it first appears. Reads
    /// the most recently changed text files first, at most `MAX_CONTENT_FILES` of up to 2 MB each. Blocking: call it
    /// off the UI thread.
    pub fn search_content(&self, phrase: &str, limit: usize) -> Vec<(Entry, usize)> {
        let needle = fold(&phrase.to_lowercase()).split_whitespace().collect::<Vec<_>>().join(" ");
        if needle.chars().count() < 4 {
            return vec![];
        }
        let mut files: Vec<Entry> = match self.entries.read() {
            Ok(e) => e.iter().filter(|e| is_text_name(&e.name_lower) && e.size <= 2 << 20).cloned().collect(),
            Err(_) => return vec![],
        };
        files.sort_by_key(|e| std::cmp::Reverse(e.modified));
        let mut out = Vec::new();
        for e in files.into_iter().take(MAX_CONTENT_FILES) {
            let Ok(text) = std::fs::read_to_string(&e.path) else { continue };
            for (i, line) in text.lines().enumerate() {
                if fold(&line.to_lowercase()).split_whitespace().collect::<Vec<_>>().join(" ").contains(&needle) {
                    out.push((e.clone(), i + 1));
                    break;
                }
            }
            if out.len() >= limit {
                break;
            }
        }
        out
    }
}

const MAX_CONTENT_FILES: usize = 3000;

const STOPWORDS: &[&str] = &["a", "o", "as", "os", "de", "do", "da", "dos", "das", "e", "em", "no", "na", "nos", "nas", "um", "uma",
    "para", "por", "com", "the", "of", "and", "in", "on", "to", "for", "an", "el", "la", "los", "las", "y", "del"];

fn significant_words(query: &str) -> Vec<String> {
    let all: Vec<String> = query.to_lowercase().split_whitespace().map(fold).collect();
    let kept: Vec<String> = all.iter().filter(|w| !STOPWORDS.contains(&w.as_str())).cloned().collect();
    if kept.is_empty() { all } else { kept }
}

/// The word is in the name, or a part of the name of 3+ letters starts the word ("out" → "outubro").
fn word_matches(word: &str, name: &str, parts: &[&str]) -> bool {
    name.contains(word) || parts.iter().any(|p| p.chars().count() >= 3 && word.starts_with(p))
}

fn is_text_name(name: &str) -> bool {
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    matches!(ext, "md" | "txt" | "org" | "rst" | "tex" | "csv" | "tsv" | "adoc" | "html" | "htm" | "json" | "toml" | "yaml" | "yml")
}

/// Lowercase without accents ("relatório" finds "relatorio").
pub(crate) fn fold(s: &str) -> String {
    s.chars().map(|c| match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' => 'a', 'é' | 'ê' | 'è' | 'ë' => 'e', 'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o', 'ú' | 'ù' | 'û' | 'ü' => 'u', 'ç' => 'c', c => c,
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_ranks_and_ignores_accents() {
        let idx = FileIndex::default();
        let mk = |p: &str, m: i64| Entry { path: p.into(), name_lower: Path::new(p).file_name().unwrap().to_string_lossy().to_lowercase(), size: 1, modified: m };
        *idx.entries.write().unwrap() = vec![
            mk("/h/notas-relatorio.md", 50), mk("/h/relatorio-set.pdf", 10), mk("/h/relatorio-out.pdf", 20), mk("/h/foto.png", 99),
        ];
        let r: Vec<String> = idx.search("relatório", 5).iter().map(|e| e.path.display().to_string()).collect();
        assert_eq!(r, vec!["/h/relatorio-out.pdf", "/h/relatorio-set.pdf", "/h/notas-relatorio.md"]);
        assert_eq!(idx.search("relatorio out", 5)[0].path.display().to_string(), "/h/relatorio-out.pdf");
        assert!(idx.search("", 5).is_empty());
        // connecting words ignored; "out" in the name stands for "outubro"; a partial match comes after
        let r: Vec<String> = idx.search("relatório de outubro", 5).iter().map(|e| e.path.display().to_string()).collect();
        assert_eq!(r[0], "/h/relatorio-out.pdf");
        assert!(r.contains(&"/h/relatorio-set.pdf".to_string()));
        assert!(idx.search("foto de outubro", 5).iter().all(|e| !e.path.ends_with("relatorio-set.pdf")));
    }

    #[test]
    fn content_search_finds_the_phrase_and_its_line() {
        let d = std::env::temp_dir().join(format!("kalango-indice-teste-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("notas.md"), "# Notas\n\nRevisar o Relatório  de Outubro antes de 20/11.\n").unwrap();
        std::fs::write(d.join("outra.md"), "nada aqui\n").unwrap();
        std::fs::write(d.join("imagem.png"), "relatório de outubro").unwrap();
        let mut out = Vec::new();
        walk(&d, 0, &mut out);
        let idx = FileIndex::default();
        *idx.entries.write().unwrap() = out;
        let r = idx.search_content("relatorio de outubro", 5);
        assert_eq!(r.len(), 1, "{r:?}");
        assert!(r[0].0.path.ends_with("notas.md"));
        assert_eq!(r[0].1, 3);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
