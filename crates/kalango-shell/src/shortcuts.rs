//! The shortcuts the person can change (Settings › Input › Shortcuts): each one an action of the compositor or a shell
//! command, with the session's default keys (sessao/niri.kdl.modelo binds the same ones). The person's changes are kept
//! in `atalhos.json` and their own shortcuts (made by request) in `atalhos-proprios.json`; the shell writes them as
//! niri binds to `atalhos.kdl`, which the session's niri.kdl includes after its own binds: a changed shortcut's new keys
//! do its action and its old keys do nothing anymore (an included bind replaces one with the same keys, and niri
//! reloads it when it changes). Super tapped alone (search) and held (voice mode) come from the compositor itself and
//! are not changed here.

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    System,
    Windows,
    Terminal,
}

pub struct Shortcut {
    pub id: &'static str,
    pub group: Group,
    pub name: &'static str,
    /// the row's glyph
    pub glyph: &'static str,
    /// niri actions, one per key (a pair: snapping left / right share the modifiers)
    pub actions: &'static [&'static str],
    pub default: &'static str,
    /// held, it does it again (niri's repeat): false for what opens or closes something
    pub repeat: bool,
}

use kalango_i18n::n_;

/// `@SHELL@` is the shell's command, `@TERMINAL@` the terminal; keys as niri writes them ("Mod+Shift+E").
pub const SHORTCUTS: &[Shortcut] = &[
    Shortcut { id: "barra-do-agente", group: Group::System, name: n_("Agent bar"), glyph: "spark", actions: &[r#"spawn "@SHELL@" "comando";"#], default: "Mod+Space", repeat: false },
    Shortcut { id: "agente", group: Group::System, name: n_("Agent side bar"), glyph: "spark", actions: &[r#"spawn "@SHELL@" "lateral" "agente";"#], default: "Mod+G", repeat: false },
    Shortcut { id: "transferencia", group: Group::System, name: n_("Clipboard"), glyph: "colar", actions: &[r#"spawn "@SHELL@" "transferencia";"#], default: "Mod+V", repeat: false },
    Shortcut { id: "captura", group: Group::System, name: n_("Screenshot"), glyph: "camera", actions: &[r#"spawn "@SHELL@" "captura";"#], default: "Print", repeat: false },
    Shortcut { id: "emoji", group: Group::System, name: n_("Emoji and symbols"), glyph: "estrela-n", actions: &[r#"spawn "@SHELL@" "emoji";"#], default: "Mod+Period", repeat: false },
    Shortcut { id: "ditado", group: Group::System, name: n_("Dictation"), glyph: "microfone", actions: &[r#"spawn "@SHELL@" "ditar";"#], default: "Mod+H", repeat: false },
    Shortcut { id: "descrever", group: Group::System, name: n_("Describe the screen"), glyph: "olho", actions: &[r#"spawn "@SHELL@" "descrever-tela";"#], default: "Mod+Alt+D", repeat: false },
    Shortcut { id: "apps", group: Group::System, name: n_("All the apps"), glyph: "apps-n", actions: &[r#"spawn "@SHELL@" "apps";"#], default: "Mod+A", repeat: false },
    Shortcut { id: "avisos", group: Group::System, name: n_("Notifications and calendar"), glyph: "sino-n", actions: &[r#"spawn "@SHELL@" "lateral" "avisos";"#], default: "Mod+N", repeat: false },
    Shortcut { id: "controles", group: Group::System, name: n_("Controls"), glyph: "ajustes", actions: &[r#"spawn "@SHELL@" "lateral" "controles";"#], default: "Mod+C", repeat: false },
    Shortcut { id: "hoje", group: Group::System, name: n_("Today"), glyph: "relogio", actions: &[r#"spawn "@SHELL@" "hoje";"#], default: "Mod+W", repeat: false },
    Shortcut { id: "privacidade", group: Group::System, name: n_("Pause sensors and agents"), glyph: "escudo", actions: &[r#"spawn "@SHELL@" "privacidade" "pausar";"#], default: "Mod+Shift+P", repeat: false },
    Shortcut { id: "bloquear", group: Group::System, name: n_("Lock the screen"), glyph: "cadeado", actions: &[r#"spawn "@SHELL@" "bloquear";"#], default: "Mod+L", repeat: false },
    Shortcut { id: "gravar", group: Group::System, name: n_("Record the screen"), glyph: "video-d", actions: &[r#"spawn "@SHELL@" "captura" "video";"#], default: "Shift+Print", repeat: false },
    Shortcut { id: "atalhos", group: Group::System, name: n_("Shortcuts sheet"), glyph: "teclado", actions: &[r#"spawn "@SHELL@" "atalhos";"#], default: "Mod+Slash", repeat: false },
    Shortcut { id: "energia", group: Group::System, name: n_("Power menu"), glyph: "desligar-d", actions: &[r#"spawn "@SHELL@" "energia";"#], default: "Mod+Shift+E", repeat: false },
    Shortcut { id: "terminal", group: Group::System, name: n_("Terminal"), glyph: "terminal", actions: &[r#"spawn "@SHELL@" "terminal";"#], default: "Mod+T", repeat: true },
    Shortcut { id: "terminal-suspenso", group: Group::Terminal, name: n_("Show and hide the drop-down Terminal"), glyph: "terminal", actions: &[r#"spawn "@SHELL@" "terminal-suspenso";"#], default: "F12", repeat: false },
    Shortcut { id: "terminal-suspenso-expandir", group: Group::Terminal, name: n_("Expand and restore the drop-down Terminal"), glyph: "expandir-altura", actions: &[r#"spawn "@SHELL@" "terminal-suspenso" "expandir";"#], default: "Shift+F12", repeat: false },
    Shortcut { id: "encaixar", group: Group::Windows, name: n_("Snap left / right"), glyph: "layout", actions: &[r#"snap-window "left";"#, r#"snap-window "right";"#], default: "Mod+Left Right", repeat: true },
    Shortcut { id: "alternar", group: Group::Windows, name: n_("Switch windows"), glyph: "apps-pontos", actions: &[r#"spawn "@SHELL@" "alternar";"#], default: "Alt+Tab", repeat: true },
    Shortcut { id: "espaco", group: Group::Windows, name: n_("Switch space"), glyph: "colunas", actions: &[r#"spawn "@SHELL@" "espacos";"#], default: "Mod+Tab", repeat: false },
    Shortcut { id: "fechar", group: Group::Windows, name: n_("Close window"), glyph: "fechar", actions: &["close-window;"], default: "Mod+Q", repeat: false },
    Shortcut { id: "minimizar", group: Group::Windows, name: n_("Minimize window"), glyph: "minimizar", actions: &["minimize-window;"], default: "Mod+M", repeat: false },
    Shortcut { id: "area", group: Group::Windows, name: n_("Show the desktop"), glyph: "monitor", actions: &[r#"spawn "@SHELL@" "area-de-trabalho";"#], default: "Mod+D", repeat: false },
    Shortcut { id: "visao", group: Group::Windows, name: n_("Overview"), glyph: "grade", actions: &[r#"spawn "@SHELL@" "visao";"#], default: "Mod+O", repeat: false },
    Shortcut { id: "layouts", group: Group::Windows, name: n_("Window layouts"), glyph: "layout", actions: &[r#"spawn "@SHELL@" "encaixe";"#], default: "Mod+Z", repeat: false },
    Shortcut { id: "maximizar", group: Group::Windows, name: n_("Maximize"), glyph: "maximizar", actions: &["maximize-column;"], default: "Mod+F", repeat: true },
    Shortcut { id: "tela-cheia", group: Group::Windows, name: n_("Full screen"), glyph: "monitor", actions: &["fullscreen-window;"], default: "Mod+Shift+F", repeat: true },
    Shortcut { id: "flutuar", group: Group::Windows, name: n_("Float window"), glyph: "painel-direita", actions: &["toggle-window-floating;"], default: "Mod+Shift+V", repeat: true },
];

/// A shortcut of the person's own (made by request): its keys run a command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Custom {
    pub keys: String,
    pub name: String,
    pub command: String,
}

/// A combination's keys: "Mod+Left Right" is two binds with the same modifiers.
pub fn keys_of(combo: &str) -> Vec<String> {
    let mut parts: Vec<&str> = combo.split('+').collect();
    let last = parts.pop().unwrap_or("");
    let mods = parts.join("+");
    last.split(' ').filter(|k| !k.is_empty()).map(|k| if mods.is_empty() { k.to_string() } else { format!("{mods}+{k}") }).collect()
}

/// How a key is written on the caps (pt-BR names for some).
pub fn caps_of(combo: &str) -> Vec<String> {
    let mut parts: Vec<&str> = combo.split('+').collect();
    let last = parts.pop().unwrap_or("");
    let mut v: Vec<String> = parts.iter().map(|m| match *m {
        "Mod" | "Super" => "Super".to_string(),
        "Ctrl" | "Control" => "Ctrl".to_string(),
        other => other.to_string(),
    }).collect();
    let key = match last {
        "Left Right" => "← →".to_string(),
        "Space" | "space" => kalango_i18n::tr("Space").to_string(),
        "Left" => "←".into(),
        "Right" => "→".into(),
        "Up" => "↑".into(),
        "Down" => "↓".into(),
        // punctuation as it is printed on the key
        "Period" | "period" => ".".into(),
        "Slash" | "slash" => "/".into(),
        "Comma" | "comma" => ",".into(),
        "Apostrophe" | "apostrophe" => "'".into(),
        "Grave" | "grave" => "`".into(),
        "Semicolon" | "semicolon" => ";".into(),
        "Equal" | "equal" => "=".into(),
        "Minus" | "minus" => "−".into(),
        "Delete" => "Del".into(),
        k if k.chars().count() == 1 => k.to_uppercase(),
        k => k.to_string(),
    };
    v.push(key);
    v
}

/// A combination as niri writes it, from what a person or the agent typed ("super+shift+b" → "Mod+Shift+B"); None if
/// it is not one (no key, or only modifiers).
pub fn normalize(combo: &str) -> Option<String> {
    let parts: Vec<&str> = combo.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    let (last, mods) = parts.split_last()?;
    let mut out: Vec<String> = Vec::new();
    for m in mods {
        let m = match m.to_lowercase().as_str() {
            "mod" | "super" | "win" | "meta" | "logo" => "Mod",
            "ctrl" | "control" => "Ctrl",
            "alt" => "Alt",
            "shift" => "Shift",
            _ => return None,
        };
        if !out.iter().any(|x| x == m) {
            out.push(m.to_string());
        }
    }
    let key = match last.to_lowercase().as_str() {
        "mod" | "super" | "ctrl" | "control" | "alt" | "shift" => return None,
        "space" | "espaço" => "Space".to_string(),
        "enter" | "return" => "Return".to_string(),
        "esc" | "escape" => "Escape".to_string(),
        k if k.chars().count() == 1 => k.to_uppercase(),
        k if k.starts_with('f') && k[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) => k.to_uppercase(),
        _ => {
            let mut c = last.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str())?
        }
    };
    out.push(key);
    Some(out.join("+"))
}

/// The keys of a GTK key event as niri writes them (the shortcut being changed: what the person pressed).
pub fn from_event(key: gtk::gdk::Key, mods: gtk::gdk::ModifierType) -> Option<String> {
    use gtk::gdk::ModifierType as M;
    let name = key.to_lower().name()?.to_string();
    if ["Super_L", "Super_R", "Control_L", "Control_R", "Alt_L", "Alt_R", "Shift_L", "Shift_R", "Meta_L", "Meta_R", "ISO_Level3_Shift"].contains(&name.as_str()) {
        return None;
    }
    let mut v = Vec::new();
    if mods.contains(M::SUPER_MASK) || mods.contains(M::META_MASK) {
        v.push("Mod");
    }
    if mods.contains(M::CONTROL_MASK) {
        v.push("Ctrl");
    }
    if mods.contains(M::ALT_MASK) {
        v.push("Alt");
    }
    if mods.contains(M::SHIFT_MASK) {
        v.push("Shift");
    }
    normalize(&format!("{}+{name}", v.join("+")).trim_start_matches('+').to_string())
}

fn config_dir() -> PathBuf {
    gtk::glib::user_config_dir().join("kalango")
}

pub fn kdl_path() -> PathBuf {
    config_dir().join("atalhos.kdl")
}

/// The person's changes: id → keys.
pub fn read_changes() -> BTreeMap<String, String> {
    std::fs::read_to_string(config_dir().join("atalhos.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn read_custom() -> Vec<Custom> {
    std::fs::read_to_string(config_dir().join("atalhos-proprios.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// The keys a shortcut has now.
pub fn keys_now(s: &Shortcut, changes: &BTreeMap<String, String>) -> String {
    changes.get(s.id).cloned().unwrap_or_else(|| s.default.to_string())
}

/// What already uses these keys (a shortcut's name, or one of the person's own), other than the shortcut `except`.
pub fn clash(combo: &str, except: &str, changes: &BTreeMap<String, String>, custom: &[Custom]) -> Option<String> {
    let want: Vec<String> = keys_of(combo).iter().map(|k| k.to_lowercase()).collect();
    if let Some(s) = SHORTCUTS.iter().filter(|s| s.id != except).find(|s| keys_of(&keys_now(s, changes)).iter().any(|k| want.contains(&k.to_lowercase()))) {
        return Some(kalango_i18n::tr(s.name).to_string());
    }
    custom.iter().filter(|c| c.name != except).find(|c| want.contains(&c.keys.to_lowercase())).map(|c| c.name.clone())
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The binds niri reads after the session's own: a changed shortcut's new keys do its action, its old keys nothing
/// (a key swapped between two shortcuts does the other one's); the person's own shortcuts run their commands.
pub fn kdl(changes: &BTreeMap<String, String>, custom: &[Custom], shell: &str, terminal: &str) -> String {
    let mut binds: BTreeMap<String, String> = BTreeMap::new();
    for s in SHORTCUTS.iter().filter(|s| changes.get(s.id).is_some_and(|k| *k != s.default)) {
        for k in keys_of(s.default) {
            binds.insert(k, "spawn \"true\";".into());
        }
    }
    for s in SHORTCUTS.iter().filter(|s| changes.get(s.id).is_some_and(|k| *k != s.default)) {
        let keys = keys_of(&keys_now(s, changes));
        for (k, a) in keys.iter().zip(s.actions.iter().cycle()) {
            let action = a.replace("@SHELL@", &esc(shell)).replace("@TERMINAL@", &esc(terminal));
            binds.insert(k.clone(), format!("{}{{ {action} }}", if s.repeat { "" } else { "repeat=false " }));
        }
    }
    for c in custom {
        binds.insert(c.keys.clone(), format!("repeat=false hotkey-overlay-title=\"{}\" {{ spawn \"sh\" \"-c\" \"{}\"; }}", esc(&c.name), esc(&c.command)));
    }
    let mut out = String::from("// KalangoOS: os atalhos de Configurações › Entrada › Atalhos (escrito pelo shell; mude por lá)\nbinds {\n");
    for (k, b) in binds {
        if b.starts_with("spawn \"true\"") {
            out += &format!("    {k} {{ {b} }}\n");
        } else {
            out += &format!("    {k} {b}\n");
        }
    }
    out += "}\n";
    out
}

fn shell_and_terminal() -> (String, String) {
    let shell = std::env::var("KALANGO_SHELL_CMD").ok().filter(|s| !s.is_empty())
        .or_else(|| std::env::current_exe().ok().map(|p| p.display().to_string())).unwrap_or_else(|| "kalango-shell".into());
    let terminal = std::env::var("KALANGO_TERMINAL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "kgx".into());
    (shell, terminal)
}

/// Keeps a change and writes the binds (niri reloads them).
pub fn save(changes: &BTreeMap<String, String>) -> Result<(), String> {
    let d = config_dir();
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    std::fs::write(d.join("atalhos.json"), json!(changes).to_string()).map_err(|e| e.to_string())?;
    write_kdl()
}

pub fn save_custom(custom: &[Custom]) -> Result<(), String> {
    let d = config_dir();
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    std::fs::write(d.join("atalhos-proprios.json"), serde_json::to_string_pretty(custom).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    write_kdl()
}

pub fn write_kdl() -> Result<(), String> {
    let (shell, terminal) = shell_and_terminal();
    std::fs::write(kdl_path(), kdl(&read_changes(), &read_custom(), &shell, &terminal)).map_err(|e| e.to_string())
}

/// For the page's subtitle: how many binds there are (these and the person's own).
pub fn count() -> usize {
    SHORTCUTS.iter().map(|s| keys_of(s.default).len()).sum::<usize>() + read_custom().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pair_of_keys_is_two_binds_with_the_same_modifiers() {
        assert_eq!(keys_of("Mod+Left Right"), vec!["Mod+Left", "Mod+Right"]);
        assert_eq!(keys_of("Print"), vec!["Print"]);
        assert_eq!(keys_of("Mod+Shift+P"), vec!["Mod+Shift+P"]);
    }

    #[test]
    fn caps_show_super_arrows_and_letters() {
        assert_eq!(caps_of("Mod+Shift+p"), vec!["Super", "Shift", "P"]);
        assert_eq!(caps_of("Mod+Period"), vec!["Super", "."]);
        assert_eq!(caps_of("Mod+Slash"), vec!["Super", "/"]);
        assert_eq!(caps_of("Mod+Left Right"), vec!["Super", "← →"]);
        assert_eq!(caps_of("Alt+Tab"), vec!["Alt", "Tab"]);
        assert_eq!(caps_of("Ctrl+Apostrophe"), vec!["Ctrl", "'"]);
    }

    #[test]
    fn what_people_and_the_agent_type_becomes_niris_keys() {
        assert_eq!(normalize("super+shift+b").as_deref(), Some("Mod+Shift+B"));
        assert_eq!(normalize("Ctrl + Alt + k").as_deref(), Some("Ctrl+Alt+K"));
        assert_eq!(normalize("Mod+f5").as_deref(), Some("Mod+F5"));
        assert_eq!(normalize("Mod+Shift"), None);
        assert_eq!(normalize("Hyper+T"), None);
    }

    #[test]
    fn a_change_moves_the_bind_and_clashes_are_found() {
        let mut c = BTreeMap::new();
        assert_eq!(clash("Mod+Q", "pausar", &c, &[]).as_deref(), Some(kalango_i18n::tr("Close window")));
        assert!(clash("Mod+Shift+K", "pausar", &c, &[]).is_none());
        // nothing changed: nothing written (the session's own binds hold)
        assert!(!kdl(&c, &[], "/x/kalango-shell", "kgx").contains("Mod+"));
        c.insert("fechar".to_string(), "Mod+Shift+Q".to_string());
        assert!(clash("Mod+Q", "pausar", &c, &[]).is_none());
        let k = kdl(&c, &[], "/x/kalango-shell", "kgx");
        assert!(k.contains("Mod+Shift+Q repeat=false { close-window; }"), "{k}");
        assert!(k.contains("Mod+Q { spawn \"true\"; }"), "{k}");
        // swapped keys: the old key does the other one
        c.insert("visao".to_string(), "Mod+Q".to_string());
        let k = kdl(&c, &[], "/x/kalango-shell", "kgx");
        assert!(k.contains(r#"Mod+Q repeat=false { spawn "/x/kalango-shell" "visao"; }"#), "{k}");
        assert!(k.contains("Mod+O { spawn \"true\"; }"), "{k}");
        let own = vec![Custom { keys: "Mod+Shift+T".into(), name: "Terminal no projeto".into(), command: "cd ~/proj && kgx".into() }];
        let k = kdl(&c, &own, "/x/kalango-shell", "kgx");
        assert!(k.contains(r#"Mod+Shift+T repeat=false hotkey-overlay-title="Terminal no projeto" { spawn "sh" "-c" "cd ~/proj && kgx"; }"#), "{k}");
        assert_eq!(clash("mod+shift+t", "", &c, &own).as_deref(), Some("Terminal no projeto"));
    }

    #[test]
    fn the_drop_down_terminal_moves_to_its_new_keys() {
        let mut c = BTreeMap::new();
        c.insert("terminal-suspenso".to_string(), "Ctrl+Grave".to_string());
        let k = kdl(&c, &[], "/x/kalango-shell", "kgx");
        assert!(k.contains(r#"Ctrl+Grave repeat=false { spawn "/x/kalango-shell" "terminal-suspenso"; }"#), "{k}");
        // its old key does nothing now; the expand one is untouched (the session's bind holds)
        assert!(k.contains("    F12 { spawn \"true\"; }"), "{k}");
        assert!(!k.contains("Shift+F12"), "{k}");
        assert_eq!(clash("Shift+F12", "", &c, &[]).as_deref(), Some(kalango_i18n::tr("Expand and restore the drop-down Terminal")));
        assert_eq!(caps_of("Shift+F12"), vec!["Shift", "F12"]);
        assert_eq!(normalize("shift+f12").as_deref(), Some("Shift+F12"));
    }

    /// The session's own binds (sessao/niri.kdl.modelo) have every shortcut's default keys doing its action.
    #[test]
    fn the_session_binds_every_default() {
        let model = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sessao/niri.kdl.modelo")).unwrap();
        for s in SHORTCUTS.iter().filter(|s| s.group == Group::Terminal) {
            for (k, a) in keys_of(s.default).iter().zip(s.actions.iter()) {
                let line = model.lines().find(|l| l.trim_start().starts_with(&format!("{k} "))).unwrap_or_else(|| panic!("{k} sem bind no modelo"));
                assert!(line.contains(a.trim_end_matches(';')), "{k}: {line}");
            }
        }
    }

    #[test]
    fn niri_accepts_the_binds() {
        let niri = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../poc/.deps/src/niri/target/release/niri");
        if !niri.exists() {
            return;
        }
        let d = std::env::temp_dir().join(format!("kalango-atalhos-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let mut c = BTreeMap::new();
        c.insert("fechar".to_string(), "Mod+Shift+Q".to_string());
        c.insert("encaixar".to_string(), "Mod+Ctrl+Left Right".to_string());
        c.insert("terminal-suspenso".to_string(), "Mod+F12".to_string());
        c.insert("terminal-suspenso-expandir".to_string(), "F11".to_string());
        let own = vec![Custom { keys: "Mod+Shift+T".into(), name: "Projeto \"x\"".into(), command: "cd ~ && echo \"oi\"".into() }];
        std::fs::write(d.join("atalhos.kdl"), kdl(&c, &own, "/x/kalango-shell", "kgx")).unwrap();
        std::fs::write(d.join("niri.kdl"), "binds {\n    Mod+Q { close-window; }\n}\ninclude \"atalhos.kdl\"\n").unwrap();
        let out = std::process::Command::new(&niri).args(["validate", "--config"]).arg(d.join("niri.kdl")).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let _ = std::fs::remove_dir_all(&d);
    }
}
