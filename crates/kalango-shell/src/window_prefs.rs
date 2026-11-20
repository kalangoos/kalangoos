//! Settings › Appearance › Dock and multitasking › Windows: whether the focus follows the pointer (niri's
//! `focus-follows-mouse`, in ~/.config/kalango/janelas.kdl, which the session's niri.kdl includes — niri reloads it at
//! once) and which side the windows' buttons are on (GNOME's `button-layout`, which GTK apps follow).

use std::path::PathBuf;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

pub fn kdl_path() -> PathBuf {
    gtk::glib::user_config_dir().join("kalango/janelas.kdl")
}

pub fn focus_follows() -> bool {
    crate::configuracoes::ajustes()["foco_segue_ponteiro"].as_bool().unwrap_or(true)
}

/// niri's part, as chosen (written at the session's start and on each change).
pub fn kdl(follows: bool) -> String {
    let mut out = String::from("// KalangoOS: Configurações › Aparência › Dock e multitarefa (escrito pelo shell; mude por lá)\n");
    if follows {
        // (no scrolling the view to a window half out of it: only the focus moves)
        out += "input {\n    focus-follows-mouse max-scroll-amount=\"0%\"\n}\n";
    }
    out
}

thread_local! {
    /// The apps whose windows do not answer now (design "App não está respondendo"): at 60 % until they do.
    static DIMMED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// niri's rule for the windows of apps that do not answer (their app ids, as niri's regex, escaped).
pub fn dim_rules(app_ids: &[String]) -> String {
    let mut out = String::new();
    for id in app_ids {
        let mut re = String::new();
        for c in id.chars() {
            if "\\.+*?()[]{}^$|".contains(c) {
                // a backslash for the regex, escaped for KDL's string
                re.extend(['\\', '\\']);
            }
            re.push(c);
        }
        out += &format!("window-rule {{\n    match app-id=\"^{re}$\"\n    opacity 0.6\n}}\n");
    }
    out
}

/// An app stops (or starts again) answering: its windows dim (or come back).
pub fn set_dimmed(app_id: &str, on: bool) {
    if app_id.is_empty() {
        return;
    }
    let changed = DIMMED.with(|d| {
        let mut d = d.borrow_mut();
        let had = d.iter().any(|x| x == app_id);
        if on && !had {
            d.push(app_id.to_string());
        } else if !on && had {
            d.retain(|x| x != app_id);
        }
        on != had
    });
    if changed {
        write_kdl();
    }
}

pub fn write_kdl() {
    let p = kdl_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let novo = kdl(focus_follows()) + &DIMMED.with(|d| dim_rules(&d.borrow()));
    if std::fs::read_to_string(&p).ok().as_deref() != Some(novo.as_str()) {
        let _ = std::fs::write(&p, novo);
    }
}

pub fn set_focus_follows(on: bool) {
    crate::configuracoes::salvar_ajuste("foco_segue_ponteiro", serde_json::json!(on));
    write_kdl();
    if debug() {
        eprintln!("debug janelas: foco segue o ponteiro {on}");
    }
}

/// Whether dragging a window to a screen edge or corner snaps it (niri's KalangoOS patch 0005).
pub fn edge_snap() -> bool {
    crate::configuracoes::ajustes()["encaixe_ao_arrastar"].as_bool().unwrap_or(true)
}

/// Tells niri (at the session's start and on each change).
pub fn apply_edge_snap() {
    let on = edge_snap();
    let r = crate::servicos::niri_acao(serde_json::json!({"SetEdgeSnap": {"on": on}}));
    if debug() {
        eprintln!("debug janelas: encaixe ao arrastar {on}{}", if r.is_err() { " (o niri não tem a ação)" } else { "" });
    }
}

pub fn set_edge_snap(on: bool) {
    crate::configuracoes::salvar_ajuste("encaixe_ao_arrastar", serde_json::json!(on));
    apply_edge_snap();
}

pub fn buttons_left() -> bool {
    crate::configuracoes::ajustes()["botoes_janela"].as_str() == Some("esquerda")
}

/// GNOME's button layout for the side.
pub fn button_layout(left: bool) -> &'static str {
    if left { "close,maximize,minimize:appmenu" } else { "appmenu:minimize,maximize,close" }
}

/// The side of the windows' buttons: kept, and given to GTK apps (not in the development session, which runs on
/// someone's own desktop, unless its profile is a test one).
pub fn set_buttons_left(left: bool) {
    crate::configuracoes::salvar_ajuste("botoes_janela", serde_json::json!(if left { "esquerda" } else { "direita" }));
    let dev = std::env::var_os("KALANGO_SHELL_DEV").is_some() && std::env::var_os("KALANGO_AREA_TESTE").is_none();
    if dev {
        if debug() {
            eprintln!("debug janelas: [sessão de teste] botões à {} não foram gravados", if left { "esquerda" } else { "direita" });
        }
        return;
    }
    let ok = std::process::Command::new("gsettings").args(["set", "org.gnome.desktop.wm.preferences", "button-layout", button_layout(left)])
        .stdin(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
    if debug() {
        eprintln!("debug janelas: botões à {}{}", if left { "esquerda" } else { "direita" }, if ok { "" } else { " (gsettings falhou)" });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn niri_gets_the_focus_rule_only_when_on() {
        assert!(super::kdl(true).contains("focus-follows-mouse"));
        assert!(!super::kdl(false).contains("focus-follows-mouse"));
        assert_eq!(super::button_layout(true), "close,maximize,minimize:appmenu");
    }

    #[test]
    fn a_window_that_does_not_answer_dims_by_its_app_id() {
        let r = super::dim_rules(&["org.exemplo.Planilha".to_string()]);
        assert_eq!(r, "window-rule {\n    match app-id=\"^org\\\\.exemplo\\\\.Planilha$\"\n    opacity 0.6\n}\n");
    }
}
