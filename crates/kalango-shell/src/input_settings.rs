//! Settings › Input (designs "Configurações · Teclado", "Teclado e atalhos", "Mouse e touchpad"): the keyboard's
//! layouts, typing and special keys, the mouse and the touchpad. Kept in ~/.config/kalango/entrada.json and written as
//! niri's own `input` section to ~/.config/kalango/entrada.kdl, which the session's niri.kdl includes: niri reloads it
//! as soon as it changes (a new layout, a faster repeat, a left-handed mouse take effect at once).
//!
//! The layouts' names come from xkeyboard-config (evdev.xml), translated by its own catalog.

use serde::{Deserialize, Serialize};
use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    pub layout: String,
    #[serde(default)]
    pub variant: String,
}

/// The keys that switch layouts (xkb's grp: options).
pub const SWITCH_KEYS: &[(&str, &str, &str)] = &[
    ("alt_shift", "grp:alt_shift_toggle", "Alt+Shift"),
    ("win_space", "grp:win_space_toggle", "Super+Space"),
    ("ctrl_shift", "grp:ctrl_shift_toggle", "Ctrl+Shift"),
    ("caps", "grp:caps_toggle", "Caps Lock"),
];

/// Where the Compose key is (xkb's compose: options); "none" has no option.
pub const COMPOSE: &[(&str, &str, &str)] = &[
    ("ralt", "compose:ralt", kalango_i18n::n_("Right Alt")),
    ("rwin", "compose:rwin", kalango_i18n::n_("Right Super")),
    ("menu", "compose:menu", kalango_i18n::n_("Menu")),
    ("rctrl", "compose:rctrl", kalango_i18n::n_("Right Ctrl")),
    ("none", "", kalango_i18n::n_("None")),
];

/// What Caps Lock does (xkb's caps: options).
pub const CAPS: &[(&str, &str, &str)] = &[
    ("normal", "", kalango_i18n::n_("Normal")),
    ("ctrl", "caps:ctrl_modifier", kalango_i18n::n_("Another Ctrl")),
    ("escape", "caps:escape", kalango_i18n::n_("Esc")),
    ("off", "caps:none", kalango_i18n::n_("Off")),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputSettings {
    pub layouts: Vec<Layout>,
    pub switch_keys: String,
    pub compose: String,
    pub caps: String,
    pub numlock: bool,
    /// ms before a held key repeats
    pub repeat_delay: u32,
    /// repeats per second
    pub repeat_rate: u32,
    /// -1 … 1 (libinput's accel speed)
    pub mouse_speed: f64,
    /// the pointer goes further when the hand is quick (libinput's adaptive profile; off: flat)
    pub mouse_accel: bool,
    pub left_handed: bool,
    /// niri's scroll-factor (1: as the device sends it)
    pub scroll_speed: f64,
    /// kinetic scrolling in the system's own lists (GTK)
    pub smooth_scroll: bool,
    pub touchpad_speed: f64,
    pub tap: bool,
    pub natural_scroll: bool,
    pub dwt: bool,
}

impl Default for InputSettings {
    fn default() -> Self {
        InputSettings {
            layouts: vec![Layout { layout: std::env::var("KALANGO_TECLADO").ok().filter(|t| !t.is_empty()).unwrap_or_else(|| "br".into()), variant: String::new() }],
            switch_keys: "alt_shift".into(),
            compose: "ralt".into(),
            caps: "normal".into(),
            numlock: true,
            repeat_delay: 600,
            repeat_rate: 25,
            mouse_speed: 0.0,
            mouse_accel: true,
            left_handed: false,
            scroll_speed: 1.0,
            smooth_scroll: true,
            touchpad_speed: 0.0,
            tap: true,
            natural_scroll: true,
            dwt: true,
        }
    }
}

fn dir() -> PathBuf {
    gtk::glib::user_config_dir().join("kalango")
}

pub fn kdl_path() -> PathBuf {
    dir().join("entrada.kdl")
}

pub fn read() -> InputSettings {
    std::fs::read(dir().join("entrada.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Keeps the settings and writes niri's input section (niri reloads it).
pub fn save(s: &InputSettings) -> Result<(), String> {
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    std::fs::write(dir().join("entrada.json"), serde_json::to_string_pretty(s).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::write(kdl_path(), kdl(s)).map_err(|e| e.to_string())
}

fn option<'a>(table: &'a [(&str, &str, &str)], id: &str) -> &'a str {
    table.iter().find(|(i, _, _)| *i == id).map(|(_, o, _)| *o).unwrap_or("")
}

/// xkb's options for these settings ("grp:alt_shift_toggle,compose:ralt").
pub fn xkb_options(s: &InputSettings) -> String {
    let mut v: Vec<&str> = Vec::new();
    if s.layouts.len() > 1 {
        v.push(option(SWITCH_KEYS, &s.switch_keys));
    }
    v.push(option(COMPOSE, &s.compose));
    v.push(option(CAPS, &s.caps));
    v.into_iter().filter(|o| !o.is_empty()).collect::<Vec<_>>().join(",")
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// niri's input section for these settings.
pub fn kdl(s: &InputSettings) -> String {
    let layouts = s.layouts.iter().map(|l| l.layout.as_str()).collect::<Vec<_>>().join(",");
    let variants = s.layouts.iter().map(|l| l.variant.as_str()).collect::<Vec<_>>().join(",");
    let mut out = String::from("// KalangoOS: Configurações › Entrada (escrito pelo shell; mude por lá)\ninput {\n    keyboard {\n        xkb {\n");
    out += &format!("            layout \"{}\"\n", esc(&layouts));
    if variants.chars().any(|c| c != ',') {
        out += &format!("            variant \"{}\"\n", esc(&variants));
    }
    let o = xkb_options(s);
    if !o.is_empty() {
        out += &format!("            options \"{}\"\n", esc(&o));
    }
    out += "        }\n";
    out += &format!("        repeat-delay {}\n        repeat-rate {}\n", s.repeat_delay.clamp(150, 2000), s.repeat_rate.clamp(1, 100));
    out += &format!("        numlock {}\n    }}\n", s.numlock);
    out += "    mouse {\n";
    out += &format!("        accel-speed {:.2}\n", s.mouse_speed.clamp(-1.0, 1.0));
    out += &format!("        accel-profile \"{}\"\n", if s.mouse_accel { "adaptive" } else { "flat" });
    if s.left_handed {
        out += "        left-handed\n";
    }
    out += &format!("        scroll-factor {:.2}\n    }}\n", s.scroll_speed.clamp(0.1, 10.0));
    out += "    touchpad {\n";
    out += &format!("        accel-speed {:.2}\n", s.touchpad_speed.clamp(-1.0, 1.0));
    for (on, flag) in [(s.tap, "tap"), (s.natural_scroll, "natural-scroll"), (s.dwt, "dwt"), (s.left_handed, "left-handed")] {
        if on {
            out += &format!("        {flag}\n");
        }
    }
    out += "    }\n}\n";
    out
}

// ---------------------------------------------------------------- the layouts' catalog

extern "C" {
    fn dgettext(domain: *const c_char, msgid: *const c_char) -> *const c_char;
}

/// A text of xkeyboard-config in the person's language.
fn xkb_tr(s: &str) -> String {
    let (Ok(d), Ok(m)) = (CString::new("xkeyboard-config"), CString::new(s)) else { return s.to_string() };
    unsafe {
        let r = dgettext(d.as_ptr(), m.as_ptr());
        if r.is_null() {
            return s.to_string();
        }
        CStr::from_ptr(r).to_string_lossy().into_owned()
    }
}

#[derive(Clone, Debug)]
pub struct CatalogEntry {
    pub layout: Layout,
    pub name: String,
}

fn tag<'a>(s: &'a str, t: &str) -> Option<&'a str> {
    let a = s.find(&format!("<{t}>"))? + t.len() + 2;
    let b = s[a..].find(&format!("</{t}>"))? + a;
    Some(s[a..b].trim())
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'")
}

/// Every layout and variant xkeyboard-config knows, with its name in the person's language.
pub fn catalog() -> Vec<CatalogEntry> {
    let Ok(xml) = std::fs::read_to_string("/usr/share/X11/xkb/rules/evdev.xml") else { return vec![] };
    let Some(lista) = xml.find("<layoutList>").map(|i| &xml[i..]) else { return vec![] };
    let mut out = Vec::new();
    for bloco in lista.split("<layout>").skip(1) {
        let bloco = bloco.split("</layout>").next().unwrap_or("");
        let cab = bloco.split("<variantList>").next().unwrap_or("");
        let (Some(nome), Some(desc)) = (tag(cab, "name"), tag(cab, "description")) else { continue };
        let desc = xkb_tr(&unescape(desc));
        out.push(CatalogEntry { layout: Layout { layout: nome.to_string(), variant: String::new() }, name: desc.clone() });
        if let Some(vl) = bloco.split("<variantList>").nth(1) {
            for v in vl.split("<variant>").skip(1) {
                if let (Some(vn), Some(vd)) = (tag(v, "name"), tag(v, "description")) {
                    out.push(CatalogEntry { layout: Layout { layout: nome.to_string(), variant: vn.to_string() }, name: xkb_tr(&unescape(vd)) });
                }
            }
        }
    }
    out
}

/// A layout's name ("Português (Brasil)"), from the catalog (else its code).
pub fn layout_name(l: &Layout, cat: &[CatalogEntry]) -> String {
    cat.iter().find(|c| c.layout == *l).map(|c| c.name.clone())
        .unwrap_or_else(|| if l.variant.is_empty() { l.layout.clone() } else { format!("{} ({})", l.layout, l.variant) })
}

/// The layout in use now (niri's index), if niri says.
pub fn current_layout() -> Option<usize> {
    let v = crate::servicos::niri_pedido(serde_json::json!("KeyboardLayouts")).ok()?;
    v["Ok"]["KeyboardLayouts"]["current_idx"].as_u64().map(|i| i as usize)
}

/// Makes a layout the one in use (niri switches it at once).
pub fn use_layout(i: usize) -> Result<(), String> {
    crate::servicos::niri_acao(serde_json::json!({ "SwitchLayout": { "layout": { "Index": i } } }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn niri_input_for_two_layouts_and_a_left_handed_mouse() {
        let mut s = InputSettings::default();
        s.layouts = vec![Layout { layout: "br".into(), variant: String::new() }, Layout { layout: "us".into(), variant: "intl".into() }];
        s.left_handed = true;
        s.mouse_accel = false;
        s.caps = "escape".into();
        let k = kdl(&s);
        assert!(k.contains("layout \"br,us\"") && k.contains("variant \",intl\""), "{k}");
        assert!(k.contains("options \"grp:alt_shift_toggle,compose:ralt,caps:escape\""), "{k}");
        assert!(k.contains("accel-profile \"flat\"") && k.contains("left-handed") && k.contains("numlock true"), "{k}");
        // one layout: no switch keys, no variant line
        s.layouts.truncate(1);
        let k = kdl(&s);
        assert!(!k.contains("grp:") && !k.contains("variant"), "{k}");
    }

    #[test]
    fn the_catalog_knows_brazil_and_its_variants() {
        let c = catalog();
        if c.is_empty() {
            return; // (no xkeyboard-config here)
        }
        assert!(c.iter().any(|e| e.layout.layout == "br" && e.layout.variant.is_empty()));
        assert!(c.iter().any(|e| e.layout.layout == "us" && e.layout.variant == "intl"));
    }

    #[test]
    fn niri_accepts_the_section() {
        // the same check the session makes on its own config (niri validate), when niri is here
        let niri = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../poc/.deps/src/niri/target/release/niri");
        if !niri.exists() {
            return;
        }
        let d = std::env::temp_dir().join(format!("kalango-entrada-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let mut s = InputSettings::default();
        s.layouts.push(Layout { layout: "us".into(), variant: "intl".into() });
        std::fs::write(d.join("entrada.kdl"), kdl(&s)).unwrap();
        std::fs::write(d.join("niri.kdl"), "include \"entrada.kdl\"\n").unwrap();
        let out = std::process::Command::new(&niri).args(["validate", "--config"]).arg(d.join("niri.kdl")).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let _ = std::fs::remove_dir_all(&d);
    }
}
