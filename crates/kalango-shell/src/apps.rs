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
        Some(p) => gtk::Image::from_file(p),
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
    let img = gtk::Image::from_file(pasta_icones().join(format!("{nome}.svg")));
    img.set_pixel_size(tamanho);
    img
}

pub fn abrir(app: &App) -> Result<(), String> {
    let ctx = gtk::gdk::Display::default().map(|d| d.app_launch_context());
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
