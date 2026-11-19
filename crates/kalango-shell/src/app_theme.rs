//! The apps look like KalangoOS (not like the system the session happens to run on). GTK 4 / libadwaita apps take
//! their colours from named colours (`window_bg_color`, `accent_bg_color`, …) that the person's gtk.css can redefine;
//! GTK 3 apps likewise (`theme_bg_color`, …). The shell writes KalangoOS's palette there, from its own tokens, in
//! the theme of the moment (dark or light, the accent), with the design's language: flat, no shadows, subtle corners.
//!
//! The sheets are `kalangoos.css` in the profile's gtk-4.0 and gtk-3.0 folders, imported at the top of their
//! gtk.css (whatever else the person wrote there stays, after it). Written again on every change of the theme; an
//! app reads them when it starts.
//!
//! Not in the development session that runs on someone's own desktop (its gtk.css is that desktop's), unless its
//! profile is a test one.

use std::path::{Path, PathBuf};

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

const IMPORT: &str = "@import url(\"kalangoos.css\");";

fn t(n: &str) -> String {
    crate::tema::token(n)
}

/// libadwaita's named colours with KalangoOS's tokens, and its shapes.
pub fn css_gtk4() -> String {
    let (base, surface, sunken, raised, border, text, muted, accent, accent_fg) =
        (t("k_base"), t("k_surface"), t("k_sunken"), t("k_raised"), t("k_border"), t("k_text"), t("k_muted"), t("k_accent"), t("k_accent_fg"));
    let (teal, gold, coral) = (t("k_teal"), t("k_gold"), t("k_coral"));
    format!(
        "/* KalangoOS: the apps' colours (written by the shell; Settings › Appearance changes them) */\n\
@define-color accent_color {accent};\n@define-color accent_bg_color {accent};\n@define-color accent_fg_color {accent_fg};\n\
@define-color destructive_color {coral};\n@define-color destructive_bg_color {coral};\n@define-color destructive_fg_color {accent_fg};\n\
@define-color success_color {teal};\n@define-color success_bg_color {teal};\n@define-color success_fg_color {accent_fg};\n\
@define-color warning_color {gold};\n@define-color warning_bg_color {gold};\n@define-color warning_fg_color {accent_fg};\n\
@define-color error_color {coral};\n@define-color error_bg_color {coral};\n@define-color error_fg_color {accent_fg};\n\
@define-color window_bg_color {surface};\n@define-color window_fg_color {text};\n\
@define-color view_bg_color {sunken};\n@define-color view_fg_color {text};\n\
@define-color headerbar_bg_color {surface};\n@define-color headerbar_fg_color {text};\n@define-color headerbar_border_color {border};\n\
@define-color headerbar_backdrop_color {surface};\n@define-color headerbar_shade_color {border};\n@define-color headerbar_darker_shade_color {border};\n\
@define-color sidebar_bg_color {base};\n@define-color sidebar_fg_color {text};\n@define-color sidebar_backdrop_color {base};\n@define-color sidebar_shade_color {border};\n\
@define-color secondary_sidebar_bg_color {base};\n@define-color secondary_sidebar_fg_color {text};\n@define-color secondary_sidebar_backdrop_color {base};\n\
@define-color card_bg_color {raised};\n@define-color card_fg_color {text};\n@define-color card_shade_color {border};\n\
@define-color dialog_bg_color {surface};\n@define-color dialog_fg_color {text};\n\
@define-color popover_bg_color {raised};\n@define-color popover_fg_color {text};\n@define-color popover_shade_color {border};\n\
@define-color thumbnail_bg_color {raised};\n@define-color thumbnail_fg_color {text};\n\
@define-color shade_color alpha(black, 0.2);\n@define-color scrollbar_outline_color transparent;\n\
/* flat, no shadows, subtle corners */\n\
window.csd, window.csd.maximized {{ box-shadow: none; }}\n\
window.csd:not(.maximized):not(.tiled):not(.fullscreen) {{ border-radius: 6px; outline: 1px solid {border}; outline-offset: -1px; }}\n\
button, entry, spinbutton, dropdown > button, combobox > button, .card, list.boxed-list, popover > contents {{ border-radius: 4px; }}\n\
button.suggested-action, button.destructive-action {{ box-shadow: none; }}\n\
popover > contents, .card {{ box-shadow: none; border: 1px solid {border}; }}\n\
.dim-label, label.subtitle {{ color: {muted}; }}\n"
    )
}

/// GTK 3's named colours (Adwaita's) with the same tokens.
pub fn css_gtk3() -> String {
    let (surface, sunken, border, text, accent, accent_fg, muted) =
        (t("k_surface"), t("k_sunken"), t("k_border"), t("k_text"), t("k_accent"), t("k_accent_fg"), t("k_muted"));
    format!(
        "/* KalangoOS: the apps' colours (written by the shell) */\n\
@define-color theme_bg_color {surface};\n@define-color theme_fg_color {text};\n@define-color theme_base_color {sunken};\n@define-color theme_text_color {text};\n\
@define-color theme_selected_bg_color {accent};\n@define-color theme_selected_fg_color {accent_fg};\n@define-color insensitive_fg_color {muted};\n\
@define-color borders {border};\n@define-color unfocused_borders {border};\n\
@define-color theme_unfocused_bg_color {surface};\n@define-color theme_unfocused_fg_color {text};\n@define-color theme_unfocused_base_color {sunken};\n\
@define-color theme_unfocused_selected_bg_color {accent};\n@define-color theme_unfocused_selected_fg_color {accent_fg};\n\
window decoration {{ box-shadow: none; border-radius: 6px; }}\n"
    )
}

/// The person's gtk.css begins with the import (added once; their own rules stay after it).
fn ensure_import(gtk_css: &Path) {
    let own = std::fs::read_to_string(gtk_css).unwrap_or_default();
    if own.lines().any(|l| l.trim() == IMPORT) {
        return;
    }
    let novo = if own.is_empty() { format!("{IMPORT}\n") } else { format!("{IMPORT}\n{own}") };
    let _ = std::fs::write(gtk_css, novo);
}

fn write_if_changed(p: &Path, text: &str) -> bool {
    if std::fs::read_to_string(p).ok().as_deref() == Some(text) {
        return false;
    }
    std::fs::write(p, text).is_ok()
}

fn config_dir() -> PathBuf {
    gtk::glib::user_config_dir()
}

/// Writes the apps' sheets as the theme is now.
pub fn apply() {
    let dev = std::env::var_os("KALANGO_SHELL_DEV").is_some() && std::env::var_os("KALANGO_AREA_TESTE").is_none();
    if dev {
        if debug() {
            eprintln!("debug tema dos apps: [sessão de desenvolvimento] não gravado (o gtk.css é o deste computador)");
        }
        return;
    }
    let mut mudou = false;
    for (pasta, css) in [("gtk-4.0", css_gtk4()), ("gtk-3.0", css_gtk3())] {
        let dir = config_dir().join(pasta);
        let _ = std::fs::create_dir_all(&dir);
        mudou |= write_if_changed(&dir.join("kalangoos.css"), &css);
        ensure_import(&dir.join("gtk.css"));
    }
    if debug() && mudou {
        eprintln!("debug tema dos apps: {} {}", if crate::tema::e_claro() { "claro" } else { "escuro" }, crate::tema::token("k_accent"));
    }
}

/// At the start and on every change of the theme.
pub fn start() {
    apply();
    crate::tema::ao_mudar(apply);
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_import_is_added_once_and_keeps_the_rest() {
        let d = std::env::temp_dir().join(format!("kalango-app-theme-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("gtk.css");
        std::fs::write(&p, "button { color: red; }\n").unwrap();
        super::ensure_import(&p);
        super::ensure_import(&p);
        let s = std::fs::read_to_string(&p).unwrap();
        assert_eq!(s.matches(super::IMPORT).count(), 1);
        assert!(s.starts_with(super::IMPORT) && s.contains("button { color: red; }"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
