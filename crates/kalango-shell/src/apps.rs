//! Apps instalados (entradas .desktop), apps fixados na dock e o ícone de cada um: o ícone do KalangoOS quando o
//! app tem um equivalente (Arquivos, Navegador, Terminal…), senão o ícone do tema do sistema.

use gtk::gio;
use gtk::prelude::*;
use std::path::PathBuf;

/// (ícone do KalangoOS, entradas .desktop equivalentes, em ordem de preferência)
const EQUIVALENTES: &[(&str, &[&str])] = &[
    ("arquivos", &["os.kalango.Arquivos", "org.gnome.Nautilus", "nautilus", "thunar", "org.kde.dolphin", "nemo"]),
    ("navegador", &["firefox_firefox", "firefox", "org.mozilla.firefox", "google-chrome", "com.google.Chrome", "chromium", "chromium-browser", "brave-browser"]),
    ("terminal", &["os.kalango.Terminal", "org.gnome.Terminal", "org.gnome.Ptyxis", "kitty", "Alacritty", "foot", "org.wezfurlong.wezterm"]),
    ("editor-de-texto", &["os.kalango.Editor", "org.gnome.TextEditor", "org.gnome.gedit", "gedit"]),
    ("musica", &["os.kalango.Musica", "spotify_spotify", "spotify", "rhythmbox", "org.gnome.Rhythmbox3"]),
    ("configuracoes", &["os.kalango.Configuracoes", "org.gnome.Settings", "gnome-control-center"]),
    ("calculadora", &["os.kalango.Calculadora", "org.gnome.Calculator"]),
    ("calendario", &["os.kalango.Calendario", "org.gnome.Calendar"]),
    ("imagens", &["os.kalango.Imagens", "org.gnome.Loupe", "org.gnome.eog", "eog"]),
    ("video", &["os.kalango.Video", "org.gnome.Totem", "vlc", "mpv"]),
    ("correio", &["thunderbird", "org.mozilla.Thunderbird", "thunderbird_thunderbird"]),
    ("monitor", &["os.kalango.Monitor", "org.gnome.SystemMonitor", "gnome-system-monitor"]),
    ("discos", &["os.kalango.Discos", "org.gnome.DiskUtility"]),
    ("camera", &["org.gnome.Snapshot", "cheese"]),
    ("documentos", &["os.kalango.Documentos", "org.gnome.Evince", "org.gnome.Papers"]),
    ("agentes", &["os.kalango.Agentes"]),
    ("central-de-apps", &["os.kalango.Apps", "org.gnome.Software", "snap-store_snap-store", "org.kde.discover"]),
];

/// Ordem padrão da dock (o que estiver instalado).
const FIXADOS: &[&str] = &["arquivos", "agentes", "navegador", "terminal", "editor-de-texto", "musica", "configuracoes"];

/// The file name of what an app runs (its command line's first word), if it has one. (Not `executable()`: GIO gives
/// NULL for an app started over D-Bus with no Exec, and the binding takes it for a path — a crash.)
pub fn executable_name(info: &impl IsA<gio::AppInfo>) -> Option<String> {
    let cmd = info.as_ref().commandline()?;
    let first = cmd.to_string_lossy().split_whitespace().next()?.trim_matches('"').to_string();
    std::path::Path::new(&first).file_name().map(|n| n.to_string_lossy().into_owned())
}

#[derive(Clone)]
pub struct App {
    pub info: gio::DesktopAppInfo,
    /// id sem ".desktop"
    pub id: String,
    pub nome: String,
    /// ícone próprio do KalangoOS (nome do arquivo em assets/icons/kalango/svg)
    pub icone_kalango: Option<&'static str>,
}

pub fn pasta_icones() -> PathBuf {
    if let Some(p) = std::env::var_os("KALANGO_ICONES") {
        return p.into();
    }
    let instalado = PathBuf::from("/usr/share/kalango/icones");
    if instalado.is_dir() {
        return instalado;
    }
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/icons/kalango/svg"))
}

fn sem_desktop(id: &str) -> String {
    id.trim_end_matches(".desktop").to_string()
}

pub fn todos() -> Vec<App> {
    let mut v: Vec<App> = gio::AppInfo::all().into_iter()
        .filter(|a| a.should_show())
        .filter_map(|a| a.downcast::<gio::DesktopAppInfo>().ok())
        .map(|info| {
            let id = sem_desktop(info.id().as_deref().unwrap_or(""));
            let icone_kalango = EQUIVALENTES.iter().find(|(_, ids)| ids.contains(&id.as_str())).map(|(k, _)| *k);
            App { nome: info.display_name().to_string(), id, info, icone_kalango }
        })
        .collect();
    // the other system's apps that KalangoOS's own replace (GNOME's Calculator, Text Editor, Files, Settings…) are
    // not offered: the grid, the search and the dock show KalangoOS's
    let ids: Vec<String> = v.iter().map(|a| a.id.clone()).collect();
    v.retain(|a| !substituido(&a.id, &ids) && !servico_do_sistema(&a.id));
    v.sort_by_key(|a| a.nome.to_lowercase());
    v
}

/// The screens of a service KalangoOS uses only as a daemon: the phone link is KDE Connect's protocol, and
/// "Dispositivos e continuidade" is its screen (its app, settings and SMS windows are not offered).
fn servico_do_sistema(id: &str) -> bool {
    id.starts_with("org.kde.kdeconnect") || id == "kcm_kdeconnect"
}

/// An app whose role KalangoOS's own app (installed, `os.kalango.*`) takes.
fn substituido(id: &str, instalados: &[String]) -> bool {
    EQUIVALENTES.iter().any(|(_, ids)| {
        ids.first().is_some_and(|p| p.starts_with("os.kalango.") && *p != id && instalados.iter().any(|i| i == p)) && ids[1..].contains(&id)
    })
}

fn dock_config() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    base.join("kalango/dock.json")
}

/// Pinned apps chosen by the person (saved list of desktop ids), if any.
fn saved_pins(todos: &[App]) -> Option<Vec<App>> {
    let text = std::fs::read_to_string(dock_config()).ok()?;
    let ids: Vec<String> = serde_json::from_str(&text).ok()?;
    Some(ids.iter().filter_map(|id| todos.iter().find(|a| &a.id == id).cloned()).collect())
}

/// Saves the pinned list (order kept).
pub fn save_pins(pins: &[App]) {
    let path = dock_config();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let ids: Vec<&str> = pins.iter().map(|a| a.id.as_str()).collect();
    if let Err(e) = std::fs::write(&path, serde_json::to_string_pretty(&ids).unwrap_or_default()) {
        eprintln!("kalango-shell: {}: {e}", path.display());
    }
}

/// Os apps fixados na dock: a lista salva pela pessoa ou, na primeira vez, um por papel (o primeiro equivalente
/// instalado).
pub fn fixados(todos: &[App]) -> Vec<App> {
    if let Some(saved) = saved_pins(todos) {
        return saved;
    }
    FIXADOS.iter().filter_map(|papel| {
        let ids = EQUIVALENTES.iter().find(|(k, _)| k == papel)?.1;
        ids.iter().find_map(|id| todos.iter().find(|a| a.id == *id)).cloned()
    }).collect()
}

/// O app de uma janela (pelo app_id que o niri informa).
pub fn da_janela<'a>(app_id: &str, todos: &'a [App]) -> Option<&'a App> {
    let a = app_id.to_lowercase();
    todos.iter().find(|x| {
        let id = x.id.to_lowercase();
        id == a || id.ends_with(&format!("_{a}")) || x.info.startup_wm_class().map(|c| c.to_lowercase() == a).unwrap_or(false)
    })
}

/// Imagem do app no tamanho pedido.
pub fn imagem(app: &App, tamanho: i32) -> gtk::Image {
    let img = match app.icone_kalango.map(|k| pasta_icones().join(format!("{k}.svg"))).filter(|p| p.is_file()) {
        Some(p) => icon_from_file(&p, tamanho),
        None => match app.info.icon() {
            Some(i) => gtk::Image::from_gicon(&i),
            None => gtk::Image::from_icon_name("application-x-executable"),
        },
    };
    img.set_pixel_size(tamanho);
    img
}

/// One of KalangoOS's own app icons, by name ("agentes", "arquivos"…).
pub fn imagem_kalango(nome: &str, tamanho: i32) -> gtk::Image {
    let img = icon_from_file(&pasta_icones().join(format!("{nome}.svg")), tamanho);
    img.set_pixel_size(tamanho);
    img
}

/// The most textures kept drawn (every app's icon at the few sizes the shell shows them is far fewer).
const ICON_CACHE_MAX: usize = 512;

thread_local! {
    /// Icons already drawn, by file and size in pixels: the dock, the grid, Alt+Tab and the menus make the same icon
    /// over and over (the dock each time an app opens or closes).
    static ICON_CACHE: std::cell::RefCell<Lru<(PathBuf, i32), gtk::gdk::Texture>> = std::cell::RefCell::new(Lru::new(ICON_CACHE_MAX));
}

/// A cache of at most `max` entries that, full, lets go of the one used longest ago.
pub(crate) struct Lru<K, V> {
    max: usize,
    tick: u64,
    map: std::collections::HashMap<K, (V, u64)>,
}

impl<K: std::hash::Hash + Eq + Clone, V: Clone> Lru<K, V> {
    pub(crate) fn new(max: usize) -> Self {
        Lru { max: max.max(1), tick: 0, map: std::collections::HashMap::new() }
    }

    pub(crate) fn get(&mut self, k: &K) -> Option<V> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(k).map(|e| {
            e.1 = t;
            e.0.clone()
        })
    }

    pub(crate) fn insert(&mut self, k: K, v: V) {
        self.tick += 1;
        if !self.map.contains_key(&k) && self.map.len() >= self.max {
            if let Some(old) = self.map.iter().min_by_key(|(_, e)| e.1).map(|(k, _)| k.clone()) {
                self.map.remove(&old);
            }
        }
        self.map.insert(k, (v, self.tick));
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.map.len()
    }
}

/// The screens' biggest scale (an icon drawn for it is sharp on all of them).
fn screen_scale() -> f64 {
    gtk::gdk::Display::default()
        .map(|d| d.monitors().iter::<gtk::gdk::Monitor>().filter_map(Result::ok).map(|m| m.scale()).fold(1.0, f64::max))
        .unwrap_or(1.0)
}

/// The pixels an icon of `size` is drawn with at `scale` (a whole number, at least 1).
pub(crate) fn icon_pixels(size: i32, scale: f64) -> i32 {
    ((f64::from(size.max(1)) * scale.max(1.0)).ceil() as i32).max(1)
}

/// An icon file drawn at the size it shows (×the screen's scale), shared by every image of it: loaded whole, the
/// 512 px SVGs took 1 MB each in memory and as much again on the GPU, for a 48 px icon. When a screen's scale changes or
/// a screen comes or goes, every icon on show is drawn again for the new scale (as the SVG loaded whole always was).
fn icon_from_file(path: &std::path::Path, size: i32) -> gtk::Image {
    let px = icon_pixels(size, screen_scale());
    let Some(t) = baked_icon(path, px) else { return gtk::Image::from_file(path) };
    let img = gtk::Image::from_paintable(Some(&t));
    watch_screen_scales();
    LIVE_ICONS.with(|l| {
        let mut l = l.borrow_mut();
        // (the images gone, dropped from time to time)
        if l.len() >= 256 && l.len().is_power_of_two() {
            l.retain(|(w, ..)| w.upgrade().is_some());
        }
        l.push((img.downgrade(), path.to_path_buf(), size, std::cell::Cell::new(px)));
    });
    img
}

/// The icon file drawn at `px`, from the cache or drawn now.
fn baked_icon(path: &std::path::Path, px: i32) -> Option<gtk::gdk::Texture> {
    let key = (path.to_path_buf(), px);
    if let Some(t) = ICON_CACHE.with(|c| c.borrow_mut().get(&key)) {
        return Some(t);
    }
    let pb = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(path, px, px, true).ok()?;
    let t = gtk::gdk::Texture::for_pixbuf(&pb);
    ICON_CACHE.with(|c| c.borrow_mut().insert(key, t.clone()));
    Some(t)
}

type LiveIcon = (gtk::glib::WeakRef<gtk::Image>, PathBuf, i32, std::cell::Cell<i32>);

thread_local! {
    /// The icons drawn from files, to draw again when the scale changes.
    static LIVE_ICONS: std::cell::RefCell<Vec<LiveIcon>> = const { std::cell::RefCell::new(Vec::new()) };
    static WATCHING_SCALES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Once: every screen's scale, and screens coming and going, redraw the icons on show.
fn watch_screen_scales() {
    if WATCHING_SCALES.with(|w| w.replace(true)) {
        return;
    }
    let Some(d) = gtk::gdk::Display::default() else { return };
    let monitors = d.monitors();
    let hook = |m: &gtk::gdk::Monitor| {
        m.connect_scale_notify(|_| redraw_icons_for_scale());
    };
    monitors.iter::<gtk::gdk::Monitor>().filter_map(Result::ok).for_each(|m| hook(&m));
    monitors.connect_items_changed(move |list, pos, _removed, added| {
        for i in pos..pos + added {
            if let Some(m) = list.item(i).and_downcast::<gtk::gdk::Monitor>() {
                hook(&m);
            }
        }
        redraw_icons_for_scale();
    });
}

/// The icons on show drawn again for the screens' scale now (only those whose pixel size changed).
fn redraw_icons_for_scale() {
    redraw_icons_at(screen_scale());
}

pub(crate) fn redraw_icons_at(scale: f64) {
    let live: Vec<(gtk::Image, PathBuf, i32, i32)> = LIVE_ICONS.with(|l| {
        let mut l = l.borrow_mut();
        l.retain(|(w, ..)| w.upgrade().is_some());
        l.iter()
            .filter_map(|(w, p, size, cur)| {
                let px = icon_pixels(*size, scale);
                (px != cur.replace(px)).then(|| w.upgrade().map(|i| (i, p.clone(), *size, px))).flatten()
            })
            .collect()
    });
    for (img, path, size, px) in live {
        if let Some(t) = baked_icon(&path, px) {
            img.set_paintable(Some(&t));
            img.set_pixel_size(size);
        }
    }
}

pub fn abrir(app: &App) -> Result<(), String> {
    let ctx = gtk::gdk::Display::default().map(|d| d.app_launch_context());
    // the shell runs with LD_PRELOAD=libgtk4-layer-shell and its apps do not need it (and the Remote's permission is refused
    // for anything preloaded): what the shell preloaded does not pass on to the apps it opens
    if let Some(c) = &ctx {
        c.unsetenv("LD_PRELOAD");
        c.unsetenv("LD_AUDIT");
    }
    app.info.launch(&[], ctx.as_ref()).map_err(|e| e.to_string())?;
    record_use(&app.id);
    Ok(())
}

// ------------------------------------------------------------------ use (for "Sugeridos agora")

fn use_file() -> PathBuf {
    // XDG state dir ($XDG_STATE_HOME or ~/.local/state)
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| gtk::glib::home_dir().join(".local/state")).join("kalango/uso-apps.json")
}

/// Every launch: which app and when (local only, the last 500), so the grid can suggest what is usually opened now.
pub fn record_use(id: &str) {
    let mut v = uses();
    let now = gtk::glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
    v.push((id.to_string(), now));
    let start = v.len().saturating_sub(500);
    let list: Vec<serde_json::Value> = v[start..].iter().map(|(id, t)| serde_json::json!([id, t])).collect();
    let f = use_file();
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&f, serde_json::Value::Array(list).to_string());
}

/// (app id, unix time) of each launch recorded, oldest first.
pub fn uses() -> Vec<(String, i64)> {
    std::fs::read_to_string(use_file()).ok().and_then(|t| serde_json::from_str::<Vec<serde_json::Value>>(&t).ok()).unwrap_or_default()
        .into_iter().filter_map(|x| Some((x[0].as_str()?.to_string(), x[1].as_i64()?))).collect()
}

thread_local! {
    /// the use record as last read, and when its file was written (the dock asks on every change of the windows)
    static USES_READ: std::cell::RefCell<Option<(std::time::SystemTime, Vec<(String, i64)>)>> = const { std::cell::RefCell::new(None) };
}

/// [`uses`], read again only when the file changed.
pub fn uses_cached() -> Vec<(String, i64)> {
    let Some(m) = std::fs::metadata(use_file()).and_then(|m| m.modified()).ok() else { return Vec::new() };
    USES_READ.with(|c| {
        if let Some((_, v)) = c.borrow().as_ref().filter(|(t, _)| *t == m) {
            return v.clone();
        }
        let v = uses();
        *c.borrow_mut() = Some((m, v.clone()));
        v
    })
}

/// The dock's "recent and suggested" apps (design "Dock e multitarefa"): those opened lately, newest first, and the one
/// usually opened around this hour (the grid's rule: the last 30 days, within an hour of it, at least twice) — leaving
/// out `skip` (the pinned and open ones) and what `known` does not know; at most `n`, the suggestion kept a place.
/// (app id, whether it is the suggestion).
pub fn pick_recent_and_suggested(uses: &[(String, i64)], skip: &[String], known: impl Fn(&str) -> bool, n: usize, now: i64, hour: i32,
    hour_of: impl Fn(i64) -> Option<i32>) -> Vec<(String, bool)> {
    let free = |id: &str| known(id) && !skip.iter().any(|s| s == id);
    let mut count: std::collections::HashMap<&str, usize> = Default::default();
    for (id, t) in uses.iter().filter(|(_, t)| now - t < 30 * 86400) {
        if hour_of(*t).is_some_and(|h| (h - hour).abs() <= 1) {
            *count.entry(id.as_str()).or_default() += 1;
        }
    }
    let mut usual: Vec<(&str, usize)> = count.into_iter().filter(|(id, c)| *c >= 2 && free(id)).collect();
    // (most launches first; the same count by name, so the choice does not change from one call to the next)
    usual.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let suggestion = usual.first().map(|(id, _)| id.to_string());
    let room = n.saturating_sub(usize::from(suggestion.is_some()));
    let mut out: Vec<(String, bool)> = Vec::new();
    for (id, _) in uses.iter().rev() {
        if out.len() >= room {
            break;
        }
        if free(id) && Some(id) != suggestion.as_ref() && !out.iter().any(|(x, _)| x == id) {
            out.push((id.clone(), false));
        }
    }
    if let Some(sug) = suggestion.filter(|_| n > 0) {
        out.push((sug, true));
    }
    out
}

/// [`pick_recent_and_suggested`] now, from the use record.
pub fn recent_and_suggested(skip: &[String], known: impl Fn(&str) -> bool, n: usize) -> Vec<(String, bool)> {
    let agora = gtk::glib::DateTime::now_local().ok();
    let hour = agora.as_ref().map(|d| d.hour()).unwrap_or(12);
    let now = agora.as_ref().map(|d| d.to_unix()).unwrap_or(0);
    pick_recent_and_suggested(&uses_cached(), skip, known, n, now, hour, |t| gtk::glib::DateTime::from_unix_local(t).ok().map(|d| d.hour()))
}

/// The grid's category (design: Criação, Desenvolvimento, Internet, Sistema, IA) from the app's desktop categories.
pub fn categoria(app: &App) -> &'static str {
    let cats = app.info.categories().map(|c| c.to_string()).unwrap_or_default();
    let texto = format!("{} {} {}", app.nome, app.id, app.info.keywords().iter().map(|k| k.to_string()).collect::<Vec<_>>().join(" ")).to_lowercase();
    let ia = ["claude", "chatgpt", "llm", "lm studio", "ollama", "agente", "kalango-agente", " ia ", "inteligência artificial", "artificial intelligence"];
    if ia.iter().any(|w| texto.contains(w)) || cats.contains("ArtificialIntelligence") {
        return "ia";
    }
    let has = |c: &str| cats.split(';').any(|x| x == c);
    if has("Development") || has("IDE") || has("TerminalEmulator") {
        "desenvolvimento"
    } else if has("Graphics") || has("AudioVideo") || has("Audio") || has("Video") || has("Photography") || has("Office") {
        "criacao"
    } else if has("Network") || has("WebBrowser") || has("Email") || has("Chat") || has("InstantMessaging") {
        "internet"
    } else if has("System") || has("Settings") || has("Utility") || has("Monitor") || has("PackageManager") {
        "sistema"
    } else {
        "outros"
    }
}

/// Busca simples: começa com > palavra começa com > contém (nome, id e palavras-chave).
pub fn buscar<'a>(todos: &'a [App], q: &str) -> Vec<&'a App> {
    let q = q.trim().to_lowercase();
    if q.is_empty() {
        return vec![];
    }
    let mut v: Vec<(u8, &App)> = todos.iter().filter_map(|a| {
        let nome = a.nome.to_lowercase();
        let extra = format!("{} {} {}", a.id.to_lowercase(), a.info.keywords().iter().map(|k| k.to_lowercase()).collect::<Vec<_>>().join(" "),
            a.info.generic_name().map(|g| g.to_lowercase()).unwrap_or_default());
        let s = if nome.starts_with(&q) { 0 } else if nome.split_whitespace().any(|w| w.starts_with(&q)) { 1 } else if nome.contains(&q) { 2 } else if extra.contains(&q) { 3 } else { return None };
        Some((s, a))
    }).collect();
    v.sort_by_key(|(s, a)| (*s, a.nome.len()));
    v.into_iter().map(|(_, a)| a).take(8).collect()
}

#[cfg(test)]
mod dock_recent_tests {
    use super::pick_recent_and_suggested as pick;

    #[test]
    fn recent_newest_first_without_pinned_and_with_the_usual_one() {
        let h = 3600;
        // "c" opened at 9h on three days; "a", "b", "d" once each, "d" last; "p" is pinned
        let day = 86400;
        let now = 40 * day + 9 * h;
        let uses: Vec<(String, i64)> = vec![
            ("c".into(), 37 * day + 9 * h), ("c".into(), 38 * day + 9 * h), ("c".into(), 39 * day + 9 * h),
            ("a".into(), now - 300), ("p".into(), now - 200), ("b".into(), now - 100), ("d".into(), now - 50), ("zz".into(), now - 10),
        ];
        let hour_of = |t: i64| Some(((t % day) / h) as i32);
        let known = |id: &str| id != "zz";
        let skip = vec!["p".to_string()];
        assert_eq!(pick(&uses, &skip, known, 3, now, 9, hour_of),
            vec![("d".to_string(), false), ("b".to_string(), false), ("c".to_string(), true)]);
        // at another hour there is no suggestion: three recent ones
        assert_eq!(pick(&uses, &skip, known, 3, now + 6 * h, 15, hour_of).iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), ["d", "b", "a"]);
        assert!(pick(&uses, &skip, known, 0, now, 9, hour_of).is_empty());
    }
}

#[cfg(test)]
mod tests_substituidos {
    use super::substituido;

    #[test]
    fn gnome_replaced_by_own() {
        let instalados: Vec<String> = ["os.kalango.Calculadora", "org.gnome.Calculator", "org.gnome.Terminal", "org.gnome.TextEditor"].map(String::from).to_vec();
        assert!(substituido("org.gnome.Calculator", &instalados));
        assert!(!substituido("os.kalango.Calculadora", &instalados));
        // (no own Terminal or Editor installed here: GNOME's stay)
        assert!(!substituido("org.gnome.Terminal", &instalados));
        assert!(!substituido("org.gnome.TextEditor", &instalados));
    }
}

#[cfg(test)]
mod service_tests {
    #[test]
    fn the_phone_service_is_not_an_app() {
        assert!(super::servico_do_sistema("org.kde.kdeconnect.app"));
        assert!(super::servico_do_sistema("kcm_kdeconnect"));
        assert!(!super::servico_do_sistema("org.kde.kdenlive"));
    }
}

#[cfg(test)]
mod icon_tests {
    #[test]
    fn icon_pixels_follow_the_scale() {
        assert_eq!(super::icon_pixels(48, 1.0), 48);
        assert_eq!(super::icon_pixels(48, 1.25), 60);
        assert_eq!(super::icon_pixels(24, 2.0), 48);
        assert_eq!(super::icon_pixels(0, 0.5), 1);
    }

    #[test]
    fn the_cache_lets_go_of_the_one_used_longest_ago() {
        let mut c = super::Lru::new(2);
        c.insert("a", 1);
        c.insert("b", 2);
        assert_eq!(c.get(&"a"), Some(1)); // "a" used now: "b" is the oldest
        c.insert("c", 3);
        assert_eq!(c.len(), 2);
        assert_eq!(c.get(&"b"), None);
        assert_eq!(c.get(&"a"), Some(1));
        assert_eq!(c.get(&"c"), Some(3));
        c.insert("c", 4); // the same key again replaces, nothing is let go
        assert_eq!((c.len(), c.get(&"a"), c.get(&"c")), (2, Some(1), Some(4)));
    }

    // The shell runs with LD_PRELOAD=libgtk4-layer-shell.so.0 (the image's script) and the apps it opens must not: kalangod
    // refuses the Remote for anything preloaded, and no app needs the shell's layer-shell library. A source check: the
    // launch itself needs a display and an installed app.
    #[test]
    fn what_the_shell_preloaded_does_not_pass_to_the_apps_it_opens() {
        let src = include_str!("apps.rs");
        let from = src.find("pub fn abrir").expect("abrir");
        let body = &src[from..from + src[from..].find("\n}\n").expect("end of abrir")];
        let launch = body.find("app.info.launch").expect("the launch");
        for var in ["LD_PRELOAD", "LD_AUDIT"] {
            let call = body.find(&format!("unsetenv(\"{var}\")")).unwrap_or_else(|| panic!("abrir must unset {var}"));
            assert!(call < launch, "{var} is unset before the app is launched");
        }
    }
}
