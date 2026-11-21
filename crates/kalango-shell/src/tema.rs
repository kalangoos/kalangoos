//! Tema (design "Configurações · Aparência" and the screens of "10 · Tema claro"): the shell's look, chosen by the
//! person — dark, light or automatic (light by day), the accent colour, how round the corners are and whether the
//! desktop's widgets sit straight on the wallpaper. Kept in ~/.config/kalango/tema.json.
//!
//! The style sheet is written in the dark tokens' colours; a theme is the same sheet with each of those colours
//! swapped for the theme's (the light tokens of the Design System, the chosen accent) and the corners scaled. It is
//! applied live: the style sheet is loaded again, every glyph the shell drew is recoloured, the wallpaper's light or
//! dark version goes up, and GTK and libadwaita apps follow (org.gnome.desktop.interface color-scheme).

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;

const ESCURO: &str = include_str!("../assets/tokens-dark.css");
const CLARO: &str = include_str!("../assets/tokens-light.css");
const ESTILO: &str = include_str!("../assets/style.css");

/// The accent colours offered (the design's swatches): (id, the dark theme's colour, the light theme's token).
pub const DESTAQUES: &[(&str, &str, &str)] = &[
    ("accent", "#A5A1E2", "k_accent"),
    ("teal", "#8ED2C3", "k_teal"),
    ("blue", "#7CC3DF", "k_blue"),
    ("coral", "#EFA2A6", "k_coral"),
    ("gold", "#E5C98C", "k_gold"),
    ("purple", "#C7A7DC", "k_purple"),
    ("cream", "#F1DDB0", "k_cream"),
];

/// The accent of the dark tokens: every colour of the shell drawn "in accent" is this one.
pub const ACENTO: &str = "#A5A1E2";

#[derive(Clone, Debug, PartialEq)]
pub struct Tema {
    /// "escuro", "claro" or "automatico"
    pub estilo: String,
    /// an id of [`DESTAQUES`]
    pub destaque: String,
    /// the corners of windows and cards, in px (the design's "Sutil" is 6)
    pub cantos: u32,
    pub widgets_transparentes: bool,
    /// the files of the Desktop folder shown on the desktop
    pub desktop_files: bool,
    /// a picture of the person's for the wallpaper (none: KalangoOS's scene, light or dark with the theme)
    pub papel: Option<String>,
    /// the pictures chosen before (the newest first)
    pub papeis: Vec<String>,
    /// the text's size, 1.0 = the design's (Accessibility goes up to 200%)
    pub texto: f64,
    /// Accessibility › high contrast: pure black (or white), the text at full strength, strong outlines
    pub contraste: bool,
}

impl Default for Tema {
    fn default() -> Tema {
        Tema { estilo: "escuro".into(), destaque: "accent".into(), cantos: 6, widgets_transparentes: true, desktop_files: true, papel: None, papeis: Vec::new(), texto: 1.0, contraste: false }
    }
}

fn caminho() -> PathBuf {
    glib::user_config_dir().join("kalango/tema.json")
}

pub fn ler() -> Tema {
    let v: Value = std::fs::read(caminho()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    let d = Tema::default();
    Tema {
        estilo: v["estilo"].as_str().filter(|s| ["escuro", "claro", "automatico"].contains(s)).unwrap_or(&d.estilo).to_string(),
        destaque: v["destaque"].as_str().filter(|s| DESTAQUES.iter().any(|(id, _, _)| id == s)).unwrap_or(&d.destaque).to_string(),
        cantos: v["cantos"].as_u64().map(|c| c.min(20) as u32).unwrap_or(d.cantos),
        widgets_transparentes: v["widgets_transparentes"].as_bool().unwrap_or(d.widgets_transparentes),
        desktop_files: v["desktop_files"].as_bool().unwrap_or(d.desktop_files),
        papel: v["papel"].as_str().filter(|p| std::path::Path::new(p).is_file()).map(str::to_string),
        papeis: v["papeis"].as_array().map(|a| a.iter().filter_map(|p| p.as_str()).filter(|p| std::path::Path::new(p).is_file()).take(5).map(str::to_string).collect())
            .unwrap_or_default(),
        texto: v["texto"].as_f64().map(|t| t.clamp(0.8, 2.0)).unwrap_or(d.texto),
        contraste: v["contraste"].as_bool().unwrap_or(false),
    }
}

pub fn salvar(t: &Tema) {
    let p = caminho();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let v = json!({"estilo": t.estilo, "destaque": t.destaque, "cantos": t.cantos, "widgets_transparentes": t.widgets_transparentes,
        "desktop_files": t.desktop_files, "papel": t.papel, "papeis": t.papeis, "texto": t.texto, "contraste": t.contraste});
    let _ = std::fs::write(p, serde_json::to_string_pretty(&v).unwrap_or_default());
}

/// Whether this theme is light now ("automático": light from 7h to 19h).
pub fn claro(t: &Tema, hora: i32) -> bool {
    match t.estilo.as_str() {
        "claro" => true,
        "automatico" => (7..19).contains(&hora),
        _ => false,
    }
}

fn hora() -> i32 {
    glib::DateTime::now_local().map(|d| d.hour()).unwrap_or(12)
}

/// "Reto", "Sutil", "Médio", "Redondo" by the corners' size.
pub fn nome_dos_cantos(px: u32) -> &'static str {
    match px {
        0 => kalango_i18n::tr("Square"),
        1..=7 => kalango_i18n::tr("Subtle"),
        8..=13 => kalango_i18n::tr("Medium"),
        _ => kalango_i18n::tr("Round"),
    }
}

fn tokens(css: &str) -> Vec<(&str, String)> {
    css.lines().filter_map(|l| {
        let r = l.trim().strip_prefix("@define-color ")?;
        let (nome, cor) = r.split_once(' ')?;
        Some((nome, cor.trim_end_matches(';').trim().to_uppercase()))
    }).collect()
}

fn rgb_255(hex: &str) -> (f64, f64, f64) {
    let c = |i: usize| f64::from(u8::from_str_radix(hex.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0));
    (c(1), c(3), c(5))
}

fn hex((r, g, b): (f64, f64, f64)) -> String {
    format!("#{:02X}{:02X}{:02X}", r.round().clamp(0.0, 255.0) as u8, g.round().clamp(0.0, 255.0) as u8, b.round().clamp(0.0, 255.0) as u8)
}

/// `a` moved towards `b` by `k` (0 – 1).
fn misturar(a: &str, b: &str, k: f64) -> String {
    let (x, y) = (rgb_255(a), rgb_255(b));
    hex((x.0 + (y.0 - x.0) * k, x.1 + (y.1 - x.1) * k, x.2 + (y.2 - x.2) * k))
}

/// Each colour of the dark sheet → this theme's.
pub fn mapa(t: &Tema, claro_agora: bool) -> HashMap<String, String> {
    let mut m = HashMap::new();
    let escuro = tokens(ESCURO);
    let claro_t = tokens(CLARO);
    let do_claro = |nome: &str| claro_t.iter().find(|(n, _)| *n == nome).map(|(_, c)| c.clone());
    if claro_agora {
        for (nome, c) in &escuro {
            if let Some(l) = do_claro(nome) {
                m.entry(c.clone()).or_insert(l);
            }
        }
        // the sheet's few colours without a token: their place in the light scale
        let texto2 = do_claro("k_text2").unwrap_or_else(|| "#3D3D4D".into());
        let muted = do_claro("k_muted").unwrap_or_else(|| "#6C6C80".into());
        for (c, l) in [("#C9CAD1", texto2.as_str()), ("#B8B9C4", texto2.as_str()), ("#6E6F7A", muted.as_str()), ("#4A4B57", "#C9C9D6"), ("#4B4C55", "#C9C9D6"),
            ("#3A3B46", "#DCDCE6"), ("#33343F", "#E4E4EC"), ("#33343E", "#E4E4EC"), ("#2E2F38", "#E8E8EF"), ("#202128", "#F5F5F9"), ("#1E1F27", "#F5F5F9"),
            ("#4A3F2A", "#F4EAD2"), ("#2A1416", "#F8E1E3"),
            // (the calendar's today column and its chosen view, design "Calendário · claro r4")
            ("#1E1F26", "#E3EDEA"), ("#2A2B33", "#E4E4EC")] {
            m.insert(c.to_string(), l.to_string());
        }
    }
    // the accent: the chosen one (its light token in the light theme), and the shades made from it
    let (_, escuro_d, token_claro) = DESTAQUES.iter().find(|(id, _, _)| *id == t.destaque).copied().unwrap_or(DESTAQUES[0]);
    let acento = if claro_agora { do_claro(token_claro).unwrap_or_else(|| escuro_d.to_string()) } else { escuro_d.to_string() };
    if acento != ACENTO || claro_agora {
        m.insert(ACENTO.into(), acento.clone());
        // its hover (lighter on dark, darker on light) and its pale tint
        m.insert("#B4B0EA".into(), misturar(&acento, if claro_agora { "#000000" } else { "#FFFFFF" }, if claro_agora { 0.12 } else { 0.18 }));
        m.insert("#C9C6F2".into(), misturar(&acento, "#FFFFFF", 0.42));
        if !claro_agora {
            // the folders are the accent: their back a darker one, their shine a lighter one, the emblem a deep one
            m.insert("#7D78CF".into(), misturar(&acento, "#000000", 0.24));
            m.insert("#C4C1F3".into(), misturar(&acento, "#FFFFFF", 0.4));
            m.insert("#2E2C52".into(), misturar(&acento, "#000000", 0.72));
        }
    }
    // high contrast: every surface pure black (white in the light theme), every text and line at full strength
    if t.contraste {
        // (the raised surfaces stay a step apart: they mark the chosen and the hovered; the lines are strong but not
        // the text's colour, since some of them are also the background of buttons)
        let (fundo, elevado, linha, frente) = if claro_agora { ("#FFFFFF", "#E4E4E4", "#505050", "#000000") } else { ("#000000", "#2B2B2B", "#A0A0A0", "#FFFFFF") };
        for (nome, c) in &escuro {
            let novo = match *nome {
                "k_base" | "k_bg" | "k_panel" | "k_surface" | "k_sunken" | "k_field" | "k_mensagem" | "k_folder" | "k_scrim" => Some(fundo),
                "k_surface2" | "k_control" | "k_control2" | "k_raised" => Some(elevado),
                "k_border" | "k_divider" => Some(linha),
                "k_text" | "k_text2" | "k_muted" | "k_dim" => Some(frente),
                _ => None,
            };
            if let Some(n) = novo {
                m.insert(c.clone(), n.to_string());
            }
        }
        for c in ["#C9CAD1", "#B8B9C4", "#6E6F7A"] {
            m.insert(c.to_string(), frente.to_string());
        }
    }
    m
}

/// The sheet's corners scaled from the design's 6 px (only the soft corners: pills and circles stay round — a radius
/// of half the rule's smallest min-width / min-height or more is a circle's or a pill's: the switch's knob, a day of the
/// calendar).
fn escalar_cantos(css: &str, cantos: u32) -> String {
    if cantos == 6 {
        return css.to_string();
    }
    let k = f64::from(cantos) / 6.0;
    let px_de = |corpo: &str, prop: &str| -> Option<f64> {
        let i = corpo.find(&format!("{prop}:"))?;
        let v = corpo[i + prop.len() + 1..].trim_start();
        v.split(|c: char| c == ';' || c == '}' || c.is_whitespace()).next()?.strip_suffix("px")?.parse().ok()
    };
    let mut out = String::with_capacity(css.len());
    let mut resto = css;
    while let Some(i) = resto.find("border-radius:") {
        // the rule this radius is in (from its "{" to its "}"): its smallest side, when it says one
        let abre = resto[..i].rfind('{').map(|a| a + 1).unwrap_or(0);
        let fecha = resto[i..].find('}').map(|f| i + f).unwrap_or(resto.len());
        let corpo = &resto[abre..fecha];
        let lado = [px_de(corpo, "min-width"), px_de(corpo, "min-height")].into_iter().flatten().fold(f64::MAX, f64::min);
        out.push_str(&resto[..i + 14]);
        resto = &resto[i + 14..];
        let fim = resto.find([';', '}']).unwrap_or(resto.len());
        let valor = &resto[..fim];
        let novo: Vec<String> = valor.split_whitespace().map(|v| match v.strip_suffix("px").and_then(|n| n.parse::<f64>().ok()) {
            Some(n) if (2.0..=8.0).contains(&n) && n * 2.0 < lado => format!("{}px", (n * k).round()),
            _ => v.to_string(),
        }).collect();
        out.push(' ');
        out.push_str(&novo.join(" "));
        if valor.ends_with(char::is_whitespace) {
            out.push(' ');
        }
        resto = &resto[fim..];
    }
    out.push_str(resto);
    out
}

/// Swaps every `#RRGGBB` of `texto` through `m`.
fn trocar_cores(texto: &str, m: &HashMap<String, String>) -> String {
    if m.is_empty() {
        return texto.to_string();
    }
    let b = texto.as_bytes();
    let mut out = String::with_capacity(texto.len());
    let mut i = 0;
    let mut desde = 0;
    while i < b.len() {
        if b[i] == b'#' && i + 7 <= b.len() && b[i + 1..i + 7].iter().all(u8::is_ascii_hexdigit) && !b.get(i + 7).is_some_and(u8::is_ascii_hexdigit) {
            if let Some(n) = m.get(&texto[i..i + 7].to_uppercase()) {
                out.push_str(&texto[desde..i]);
                out.push_str(n);
                i += 7;
                desde = i;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&texto[desde..]);
    out
}

/// The light theme's fonts (design: "01 · Fundamentos" › "Cores e fontes · tema claro"): Roboto for the interface and
/// Ubuntu Bold for titles and big numbers, in place of Inter and URW Gothic Demi. Roboto has no semibold: a rule's 600 is
/// its Medium (500); a display rule's is Ubuntu's Bold (700).
fn fontes_claras(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut resto = css;
    while let Some(a) = resto.find('{') {
        let f = resto[a..].find('}').map(|f| a + f + 1).unwrap_or(resto.len());
        out.push_str(&resto[..a]);
        let regra = &resto[a..f];
        let display = regra.contains("URW Gothic");
        let peso = if display { "font-weight: 700" } else { "font-weight: 500" };
        out.push_str(&regra.replace("\"Inter\", \"Roboto\"", "\"Roboto\", \"Inter\"").replace("\"URW Gothic\"", "\"Ubuntu\"")
            .replace("font-weight: 600", peso));
        resto = &resto[f..];
    }
    out.push_str(resto);
    out
}

/// The light theme's panels are opaque (side bars, power menu, switcher, gallery…): on a light sheet a dark wallpaper
/// would show through the text as stains. The dark theme keeps their 90–97 %.
fn paineis_opacos(css: &str) -> String {
    let mut out = css.to_string();
    // (the veil under the command bar: lighter on a light sheet, as the design "Busca universal · claro r3")
    out = out.replace(".fundo { background: alpha(@k_scrim, 0.65); }", ".fundo { background: alpha(@k_scrim, 0.42); }");
    for fundo in ["@k_sunken", "#18191E"] {
        for a in ["0.97", "0.92", "0.9"] {
            out = out.replace(&format!("alpha({fundo}, {a})"), fundo);
        }
    }
    out
}

/// The accent as text colour now (design: inline code and links in a chat answer): the accent itself on the dark
/// theme, a darker one on the light (the green #34CB79 reads as #248F55 on white).
pub fn destaque_texto() -> String {
    let a = cor(ACENTO);
    if e_claro() { misturar(&a, "#000000", 0.3) } else { a }
}

/// The interface's font now (for what is drawn by hand): Inter, or Roboto in the light theme.
pub fn fonte_ui() -> &'static str {
    if e_claro() { "Roboto" } else { "Inter" }
}

/// The titles' and big numbers' font now: URW Gothic, or Ubuntu in the light theme.
pub fn fonte_display() -> &'static str {
    if e_claro() { "Ubuntu" } else { "URW Gothic" }
}

/// The whole style sheet for this theme.
pub fn css(t: &Tema, claro_agora: bool) -> String {
    let m = mapa(t, claro_agora);
    let estilo = if claro_agora { paineis_opacos(ESTILO) } else { ESTILO.to_string() };
    let mut s = trocar_cores(&format!("{ESCURO}\n{estilo}"), &m);
    if claro_agora {
        s = fontes_claras(&s);
    }
    s = escalar_cantos(&s, t.cantos);
    if !t.widgets_transparentes {
        // the desktop's widgets on cards of their own, not straight on the wallpaper
        s += "\n.desk-widget { background: alpha(@k_surface, 0.94); border: 1px solid @k_border; border-radius: 8px; padding: 12px; }\n";
    }
    // the selectors' arrow: the design's chevron (GTK's own is a thinner, smaller one), in this theme's muted colour
    if let Some(f) = chevron(m.get("#8E8F99").map(String::as_str).unwrap_or("#8E8F99")) {
        s += &format!("\ndropdown.cfg-seletor arrow, dropdown.central-escolha arrow {{ -gtk-icon-source: none; background: url(\"file://{}\") center / 16px 16px no-repeat; min-width: 16px; min-height: 16px; }}\n", f.display());
    }
    s
}

/// The design's chevron (down) in a colour, as a file GTK's style sheet can name (one per colour, in the runtime folder).
/// (A pixel higher than in the design's 16 px frame: drawn as the arrow's background it lands a pixel lower.)
fn chevron(cor: &str) -> Option<std::path::PathBuf> {
    let pasta = glib::user_runtime_dir().join("kalango");
    let f = pasta.join(format!("chevron-baixo-v2-{}.svg", cor.trim_start_matches('#').to_uppercase()));
    if !f.is_file() {
        std::fs::create_dir_all(&pasta).ok()?;
        let svg = format!("<svg width=\"16\" height=\"16\" viewBox=\"0 0 16 16\" fill=\"none\" xmlns=\"http://www.w3.org/2000/svg\"><path transform=\"translate(4 5)\" d=\"M0 0L4.5 4.5L9 0\" fill=\"none\" stroke=\"{cor}\" stroke-width=\"1.5\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/></svg>\n");
        std::fs::write(&f, svg).ok()?;
    }
    Some(f)
}

thread_local! {
    static ATUAL: RefCell<(Tema, bool, HashMap<String, String>)> = RefCell::new((Tema::default(), false, HashMap::new()));
    static PROVEDOR: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
    /// every glyph the shell drew (weak): recoloured when the theme changes
    static GLIFOS: RefCell<HashMap<usize, (glib::WeakRef<gtk::Widget>, Box<dyn Fn(&gtk::Widget)>)>> = RefCell::default();
    static OUVINTES: RefCell<Vec<Box<dyn Fn()>>> = RefCell::default();
    static GERACAO: Cell<u64> = const { Cell::new(0) };
}

/// A colour of the dark sheet as this theme shows it.
pub fn cor(hex: &str) -> String {
    ATUAL.with(|a| a.borrow().2.get(&hex.to_uppercase()).cloned()).unwrap_or_else(|| hex.to_string())
}

/// Every colour of a drawing (an SVG of the dark sheet's colours) as this theme shows it.
pub fn recolor_svg(svg: &str) -> String {
    ATUAL.with(|a| trocar_cores(svg, &a.borrow().2))
}

/// The theme as it is now.
pub fn atual() -> Tema {
    ATUAL.with(|a| a.borrow().0.clone())
}

/// A token's colour as this theme shows it now (`k_surface` → "#1C1D23" dark, its light colour when light).
pub fn token(nome: &str) -> String {
    tokens(ESCURO).into_iter().find(|(n, _)| *n == nome).map(|(_, c)| cor(&c)).unwrap_or_else(|| "#808080".into())
}

/// A light token's colour (`k_teal` → "#1A6E5D").
pub fn token_claro(nome: &str) -> Option<String> {
    tokens(CLARO).into_iter().find(|(n, _)| *n == nome).map(|(_, c)| c)
}

/// A colour of the dark sheet as this theme shows it, for cairo (0 – 1 each).
pub fn rgb(hex: &str) -> (f64, f64, f64) {
    let (r, g, b) = self::rgb_255(&cor(hex));
    (r / 255.0, g / 255.0, b / 255.0)
}

/// Whether the shell is light now.
pub fn e_claro() -> bool {
    ATUAL.with(|a| a.borrow().1)
}

/// Bumped on each change (drawings cache by it).
pub fn geracao() -> u64 {
    GERACAO.with(Cell::get)
}

/// A glyph to recolour when the theme changes (`refazer` draws it again with [`cor`]).
/// One entry a widget: the last one registered (its colour now) is kept.
pub fn registrar(w: &impl IsA<gtk::Widget>, refazer: impl Fn(&gtk::Widget) + 'static) {
    let w = w.upcast_ref::<gtk::Widget>();
    GLIFOS.with(|g| {
        let mut g = g.borrow_mut();
        if g.len() % 512 == 511 {
            g.retain(|_, (w, _)| w.upgrade().is_some());
        }
        g.insert(w.as_ptr() as usize, (w.downgrade(), Box::new(refazer)));
    });
}

/// Called after each change (the settings' preview, the dock…).
pub fn ao_mudar(f: impl Fn() + 'static) {
    OUVINTES.with(|o| o.borrow_mut().push(Box::new(f)));
}

/// Loads the style sheet at start (the provider is kept for the changes).
pub fn iniciar(display: &gdk::Display) {
    let t = ler();
    let c = claro(&t, hora());
    let p = gtk::CssProvider::new();
    p.load_from_string(&css(&t, c));
    // above the person's own ~/.config/gtk-4.0/gtk.css (USER priority): a desktop theme from elsewhere ("button { border-radius: 12px }")
    // must not redraw the system's interface
    gtk::style_context_add_provider_for_display(display, &p, gtk::STYLE_PROVIDER_PRIORITY_USER + 1);
    let m = mapa(&t, c);
    aplicar_texto(t.texto);
    ATUAL.with(|a| *a.borrow_mut() = (t, c, m));
    PROVEDOR.with(|x| *x.borrow_mut() = Some(p));
    seguir_apps(c);
}

/// The text's size: the shell's (GTK's font resolution) and, outside the development session, the apps' (GNOME's
/// text scaling).
fn aplicar_texto(f: f64) {
    if let Some(s) = gtk::Settings::default() {
        s.set_gtk_xft_dpi((96.0 * 1024.0 * f).round() as i32);
    }
    if std::env::var_os("KALANGO_SHELL_DEV").is_some() {
        return;
    }
    if let Some(fonte) = gio::SettingsSchemaSource::default() {
        if fonte.lookup("org.gnome.desktop.interface", true).is_some_and(|s| s.has_key("text-scaling-factor")) {
            let s = gio::Settings::new("org.gnome.desktop.interface");
            if (s.double("text-scaling-factor") - f).abs() > 0.001 {
                let _ = s.set_double("text-scaling-factor", f);
            }
        }
    }
}

/// The picture the person chose for the wallpaper, if any.
pub fn papel_escolhido() -> Option<PathBuf> {
    ATUAL.with(|a| a.borrow().0.papel.clone()).map(PathBuf::from)
}

/// Applies a theme now (saved by the caller); nothing happens when it looks the same.
pub fn aplicar(t: &Tema) {
    let c = claro(t, hora());
    if ATUAL.with(|a| { let a = a.borrow(); a.0 == *t && a.1 == c }) {
        return;
    }
    let (claro_antes, antes) = ATUAL.with(|a| (a.borrow().1, a.borrow().0.clone()));
    let m = mapa(t, c);
    ATUAL.with(|a| *a.borrow_mut() = (t.clone(), c, m));
    GERACAO.with(|g| g.set(g.get() + 1));
    if let Some(p) = PROVEDOR.with(|p| p.borrow().clone()) {
        p.load_from_string(&css(t, c));
    }
    // (taken out while they run: redrawing one may register another)
    let glifos = GLIFOS.with(|g| g.take());
    let vivos: HashMap<_, _> = glifos.into_iter().filter(|(_, (w, f))| match w.upgrade() {
        Some(w) => {
            f(&w);
            true
        }
        None => false,
    }).collect();
    GLIFOS.with(|g| {
        let mut g = g.borrow_mut();
        let novos = std::mem::take(&mut *g);
        *g = vivos;
        g.extend(novos);
    });
    if (antes.texto - t.texto).abs() > 0.001 {
        aplicar_texto(t.texto);
    }
    if claro_antes != c || antes.papel != t.papel {
        crate::papel::reaplicar();
    }
    if claro_antes != c {
        seguir_apps(c);
    }
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug tema: {} {} cantos {} widgets {}", if c { "claro" } else { "escuro" }, t.destaque, t.cantos,
            if t.widgets_transparentes { "transparentes" } else { "em cartões" });
    }
    let ouvintes = OUVINTES.with(|o| o.take());
    for f in &ouvintes {
        f();
    }
    OUVINTES.with(|o| {
        let mut o = o.borrow_mut();
        let novos = std::mem::take(&mut *o);
        *o = ouvintes;
        o.extend(novos);
    });
}

/// Every minute: "automático" turns light at 7h and dark at 19h.
pub fn vigiar() {
    let t = ATUAL.with(|a| a.borrow().0.clone());
    if t.estilo == "automatico" {
        aplicar(&t);
    }
}

/// GTK and libadwaita apps follow the theme (GNOME's setting, where it exists).
fn seguir_apps(claro: bool) {
    // the development session runs on someone's desktop: its settings are that desktop's, and stay as they are
    if std::env::var_os("KALANGO_SHELL_DEV").is_some() {
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug tema: [sessão de teste] color-scheme {} não foi gravado", if claro { "prefer-light" } else { "prefer-dark" });
        }
        return;
    }
    let Some(fonte) = gio::SettingsSchemaSource::default() else { return };
    if fonte.lookup("org.gnome.desktop.interface", true).is_some_and(|s| s.has_key("color-scheme")) {
        let s = gio::Settings::new("org.gnome.desktop.interface");
        let quer = if claro { "prefer-light" } else { "prefer-dark" };
        if s.string("color-scheme") != quer {
            let _ = s.set_string("color-scheme", quer);
        }
    }
}

#[cfg(test)]
mod fluxos_cores {
    use super::*;

    /// The agents platform's screens (the style sheet's "fluxos:" blocks) use only colours the light theme knows: a
    /// token's dark value or one of the few mapped ones. Any other hex stays dark in the light theme (a chosen card
    /// went dark and unreadable on white).
    #[test]
    fn fluxos_blocks_use_only_mapped_colours() {
        let t = Tema::default();
        let m = mapa(&t, true);
        let start = ESTILO.find("/* ── fluxos:").unwrap_or(ESTILO.len());
        let re = |s: &str| -> Vec<String> {
            let b = s.as_bytes();
            let mut v = Vec::new();
            for i in 0..b.len() {
                if b[i] == b'#' && i + 7 <= b.len() && b[i + 1..i + 7].iter().all(u8::is_ascii_hexdigit) && !b.get(i + 7).is_some_and(u8::is_ascii_hexdigit) {
                    v.push(s[i..i + 7].to_uppercase());
                }
            }
            v
        };
        let unknown: Vec<String> = re(&ESTILO[start..]).into_iter().filter(|c| !m.contains_key(c)).collect();
        assert!(unknown.is_empty(), "colours the light theme does not map: {unknown:?}");
    }

    /// GTK's CSS has no `:empty` (or other web-only pseudo-classes): it logs "Unknown name of pseudo-class" and drops
    /// the rule. Hide such widgets in code (superficies::hide_when_empty).
    #[test]
    fn the_style_sheet_uses_only_gtk_pseudo_classes() {
        for bad in [":empty", ":first-of-type", ":last-of-type", ":nth-of-type", ":has("] {
            assert!(!ESTILO.contains(bad), "style.css uses {bad}, which GTK does not know");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_light_theme_swaps_the_dark_tokens_for_the_light_ones() {
        let t = Tema { estilo: "claro".into(), ..Tema::default() };
        let m = mapa(&t, true);
        assert_eq!(m.get("#16171B").map(String::as_str), Some("#F0F0F5"));
        assert_eq!(m.get("#E2E2E6").map(String::as_str), Some("#322153"));
        // the default accent is the light theme's green
        assert_eq!(m.get(ACENTO).map(String::as_str), Some("#34CB79"));
        let css = css(&t, true);
        assert!(!css.contains("#16171B") && css.contains("#F0F0F5"));
    }

    #[test]
    fn the_dark_theme_with_another_accent_only_changes_the_accent() {
        let t = Tema { destaque: "teal".into(), ..Tema::default() };
        let m = mapa(&t, false);
        assert_eq!(m.get(ACENTO).map(String::as_str), Some("#8ED2C3"));
        assert!(!m.contains_key("#16171B"));
        assert!(mapa(&Tema::default(), false).is_empty());
    }

    #[test]
    fn corners_scale_the_soft_ones_and_keep_pills_round() {
        let s = escalar_cantos(".a { border-radius: 6px; } .b { border-radius: 99px; } .c { border-radius: 6px 0 0 6px; } .d { border-radius: 4px }", 12);
        assert!(s.contains(".a { border-radius: 12px;"));
        assert!(s.contains(".b { border-radius: 99px;"));
        assert!(s.contains(".c { border-radius: 12px 0 0 12px;"));
        assert!(s.contains(".d { border-radius: 8px }"));
        // a 16 px knob with an 8 px radius is a circle: it stays one with smaller corners
        let s = escalar_cantos("switch > slider { min-width: 16px; min-height: 16px; border-radius: 8px; } .e { min-height: 28px; border-radius: 6px; }", 3);
        assert!(s.contains("border-radius: 8px;"), "{s}");
        assert!(s.contains(".e { min-height: 28px; border-radius: 3px;"), "{s}");
    }

    #[test]
    fn the_light_theme_writes_in_roboto_and_ubuntu() {
        let s = fontes_claras("window { font-family: \"Inter\", \"Roboto\", sans-serif; } .a { font-weight: 600; } .d { font-family: \"URW Gothic\", sans-serif; font-weight: 600; }");
        assert!(s.contains("window { font-family: \"Roboto\", \"Inter\", sans-serif; }"), "{s}");
        assert!(s.contains(".a { font-weight: 500; }"), "{s}");
        assert!(s.contains(".d { font-family: \"Ubuntu\", sans-serif; font-weight: 700; }"), "{s}");
    }

    #[test]
    fn light_panels_are_opaque() {
        let s = css(&Tema { estilo: "claro".into(), ..Tema::default() }, true);
        assert!(!s.contains("alpha(#FFFFFF, 0.97)") && !s.contains("alpha(@k_sunken, 0.97)"), "a light panel lets the wallpaper through");
        assert!(css(&Tema::default(), false).contains("alpha(@k_sunken, 0.97)"));
    }

    #[test]
    fn automatic_is_light_by_day() {
        let t = Tema { estilo: "automatico".into(), ..Tema::default() };
        assert!(claro(&t, 7) && claro(&t, 18) && !claro(&t, 19) && !claro(&t, 3));
    }

    #[test]
    fn colours_are_swapped_whole_and_in_any_case() {
        let mut m = HashMap::new();
        m.insert("#A5A1E2".to_string(), "#8ED2C3".to_string());
        assert_eq!(trocar_cores("color: #a5a1e2; x: #A5A1E2FF; y: alpha(#A5A1E2, 0.2)", &m), "color: #8ED2C3; x: #A5A1E2FF; y: alpha(#8ED2C3, 0.2)");
    }
}
