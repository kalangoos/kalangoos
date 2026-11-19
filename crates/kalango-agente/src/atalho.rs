//! "Criar atalho por pedido" (Settings › Input): what the local model understood of the request (keys, name, the kind
//! of action and its target) becomes a real command here, from what exists on this computer: the folder as it is on
//! disk (the request's own words win over a translated guess), the app's own launcher, a web address.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn terminal() -> String {
    std::env::var("KALANGO_TERMINAL").ok().filter(|t| !t.is_empty()).unwrap_or_else(|| {
        ["kgx", "ptyxis", "gnome-terminal", "konsole", "xterm"].iter().find(|t| which(t)).map(|t| t.to_string()).unwrap_or_else(|| "x-terminal-emulator".into())
    })
}

fn which(p: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|ps| std::env::split_paths(&ps).any(|d| d.join(p).is_file()))
}

/// A single-quoted shell word.
fn q(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// A folder from what was said: as written (from home, or absolute), else a path the request itself names, else the
/// same names found case-insensitively under home.
pub fn folder(target: &str, request: &str) -> Option<PathBuf> {
    let h = home();
    let as_path = |t: &str| -> PathBuf {
        let t = t.trim().trim_matches(|c| c == '"' || c == '\'' || c == '“' || c == '”');
        let t = t.strip_prefix("~/").or_else(|| t.strip_prefix("$HOME/")).unwrap_or(t);
        if Path::new(t).is_absolute() { PathBuf::from(t) } else { h.join(t) }
    };
    let p = as_path(target);
    if p.is_dir() {
        return Some(p);
    }
    // the request's own words: the longest run of them that is a folder under home
    let words: Vec<&str> = request.split_whitespace().collect();
    let mut best: Option<PathBuf> = None;
    for i in 0..words.len() {
        for j in (i + 1..=words.len()).rev() {
            let cand = as_path(&words[i..j].join(" "));
            if cand.is_dir() && cand != h && best.as_ref().is_none_or(|b| cand.as_os_str().len() > b.as_os_str().len()) {
                best = Some(cand);
            }
        }
    }
    if best.is_some() {
        return best;
    }
    // each part of the target, case-insensitively
    let mut cur = h.clone();
    for part in target.trim().trim_start_matches("~/").split('/').filter(|p| !p.is_empty()) {
        let found = std::fs::read_dir(&cur).ok()?.flatten().map(|e| e.path()).find(|e| e.is_dir() && e.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase() == part.to_lowercase()))?;
        cur = found;
    }
    (cur != h).then_some(cur)
}

/// An app's desktop id from its name (the .desktop files the system knows).
fn app_id(name: &str) -> Option<String> {
    let want = name.trim().to_lowercase();
    let mut dirs: Vec<PathBuf> = std::env::var_os("XDG_DATA_DIRS").map(|d| std::env::split_paths(&d).collect()).unwrap_or_else(|| vec!["/usr/local/share".into(), "/usr/share".into()]);
    dirs.insert(0, std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share")));
    let mut best: Option<(usize, String)> = None;
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d.join("applications")) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "desktop") {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            if text.lines().any(|l| l.trim() == "NoDisplay=true") {
                continue;
            }
            let names: Vec<String> = text.lines().filter(|l| l.starts_with("Name=") || l.starts_with("Name[")).filter_map(|l| l.split_once('=').map(|(_, v)| v.to_lowercase())).collect();
            let id = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            // the same name, a name that has it, or the same first letters (another language's name: "Calculadora", "Calculator")
            let prefix = |n: &str| n.chars().zip(want.chars()).take_while(|(a, b)| a == b).count() >= 5;
            let score = if names.iter().any(|n| *n == want) { 3 } else if names.iter().any(|n| n.contains(&want)) || id.to_lowercase().contains(&want) { 2 }
                else if names.iter().any(|n| prefix(n)) { 1 } else { 0 };
            if score > 0 && best.as_ref().is_none_or(|(s, _)| score > *s) {
                best = Some((score, id));
            }
        }
    }
    best.map(|(_, id)| id)
}

/// The kind of action: the request's own words decide when they are clear (a small model mixes them up), else the
/// model's reading.
fn action<'a>(model: &'a str, request: &str) -> &'a str {
    let r = request.to_lowercase();
    let has = |ws: &[&str]| ws.iter().any(|w| r.contains(w));
    if has(&["terminal", "console", "konsole"]) {
        "terminal_in_folder"
    } else if has(&["http", "www.", ".com", ".br", ".org", " site", "página", "pagina", "website"]) {
        "open_website"
    } else if has(&["pasta", "diretório", "diretorio", "folder"]) {
        "open_folder"
    } else if has(&["roda ", "rodar ", "executa", "comando "]) {
        "run_command"
    } else if model == "terminal_in_folder" || model.is_empty() {
        "open_app"
    } else {
        model
    }
}

/// The shortcut from what the model understood: {keys, name, command}, or None when it cannot be made.
pub fn montar(v: &Value, request: &str) -> Option<Value> {
    let keys = v["keys"].as_str()?.trim().to_string();
    let name = v["name"].as_str()?.trim().to_string();
    let target = v["target"].as_str().unwrap_or("").trim().to_string();
    if keys.is_empty() || name.is_empty() {
        return None;
    }
    let command = match action(v["action"].as_str().unwrap_or(""), request) {
        "terminal_in_folder" => {
            let f = folder(&target, request).unwrap_or_else(home);
            format!("cd {} && exec {}", q(&f.display().to_string()), terminal())
        }
        "open_folder" => format!("xdg-open {}", q(&folder(&target, request)?.display().to_string())),
        "open_website" => {
            let t = target.trim();
            let url = if t.starts_with("http://") || t.starts_with("https://") { t.to_string() } else if t.contains('.') { format!("https://{t}") } else { return None };
            format!("xdg-open {}", q(&url))
        }
        "open_app" => format!("gtk-launch {}", q(&app_id(&target)?)),
        // a "command" that is no program here is an app's name
        "run_command" if !target.is_empty() && target.split_whitespace().next().is_some_and(which) => target,
        "run_command" if !target.is_empty() => format!("gtk-launch {}", q(&app_id(&target)?)),
        _ => return None,
    };
    Some(json!({"keys": keys, "name": name, "command": command}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_translated_folder_is_found_by_the_requests_own_words() {
        let h = std::env::temp_dir().join(format!("kalango-atalho-{}", std::process::id()));
        std::fs::create_dir_all(h.join("Documentos/repositorios")).unwrap();
        std::env::set_var("HOME", &h);
        // the model wrote "Documents"; the request says "Documentos/repositorios"
        assert_eq!(folder("Documents/repositorios", "Super+Shift+T abre o terminal na pasta Documentos/repositorios"), Some(h.join("Documentos/repositorios")));
        assert_eq!(folder("documentos/REPOSITORIOS", "abre o terminal"), Some(h.join("Documentos/repositorios")));
        let v = montar(&json!({"keys": "Mod+Shift+T", "name": "Terminal no projeto", "action": "terminal_in_folder", "target": "Documents/repositorios"}), "terminal em Documentos/repositorios").unwrap();
        assert!(v["command"].as_str().unwrap().starts_with(&format!("cd '{}/Documentos/repositorios' && exec ", h.display())), "{v}");
        let w = montar(&json!({"keys": "Ctrl+Alt+M", "name": "Gmail", "action": "open_website", "target": "mail.google.com"}), "").unwrap();
        assert_eq!(w["command"], "xdg-open 'https://mail.google.com'");
        assert!(montar(&json!({"keys": "Mod+X", "name": "x", "action": "open_website", "target": "gmail"}), "").is_none());
        let _ = std::fs::remove_dir_all(&h);
    }
}
