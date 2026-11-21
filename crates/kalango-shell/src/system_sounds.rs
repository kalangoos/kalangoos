//! The system's sounds in the shell: which file (the theme the person uses, GTK's `gtk-sound-theme-name` — the image
//! sets `kalango` —, then KalangoOS's own, then `freedesktop`; in each, `~/.local/share/sounds` first, then the
//! system's data folders; the development session uses the repository's theme), played by `kalango-sons` inside this
//! process, and when each one plays (the preferences, Do not disturb, once a session).
//!
//! In the session test nothing reaches the speakers: `KALANGO_SONS=nao` stops before this module
//! (`configuracoes_som::play_at` logs `debug som: tocaria <id>`).

use kalango_sons::{resolve_in, OWN_THEME};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The theme the person uses: GTK's setting (the image's settings.ini says `kalango`); KalangoOS's own in the
/// development session and before GTK starts.
pub fn theme_name() -> String {
    if std::env::var_os("KALANGO_SHELL_DEV").is_some() || !gtk::is_initialized_main_thread() {
        return OWN_THEME.into();
    }
    gtk::Settings::default().and_then(|s| s.gtk_sound_theme_name()).map(|s| s.to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| OWN_THEME.into())
}

/// Where sound themes are: the development session's repository first, then `~/.local/share/sounds`, then the
/// system's data folders.
fn search_bases() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if std::env::var_os("KALANGO_SHELL_DEV").is_some() {
        v.push(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/sounds")).to_path_buf());
    }
    v.push(gtk::glib::user_data_dir().join("sounds"));
    v.extend(gtk::glib::system_data_dirs().into_iter().map(|d| d.join("sounds")));
    v
}

pub fn resolve(id: &str) -> Option<PathBuf> {
    resolve_in(id, &theme_name(), &search_bases())
}

/// Plays the theme's sound for this event ID at this volume (0 – 1), in the background. How long it plays, when there
/// is a file for it.
pub fn play(id: &str, volume: f64) -> Option<Duration> {
    let Some(path) = resolve(id) else {
        if crate::configuracoes::depurando() {
            eprintln!("debug som: {id} não existe no tema");
        }
        return None;
    };
    if crate::configuracoes::depurando() {
        eprintln!("debug som: tocando {id} ({})", path.display());
    }
    kalango_sons::play(id, &path, volume, "kalango-shell")
}

// ---------------------------------------------------------------------------------------------- when they play

/// Whether an agent's sound plays: Agents › Settings › Notifications › sound, and Do not disturb when the agents
/// respect it (an approval still passes when Settings › Notifications lets approvals through).
pub fn agent_sound_allowed(pref_sound: bool, dnd: bool, respect_dnd: bool, approval: bool, approvals_break: bool) -> bool {
    pref_sound && !(dnd && respect_dnd && !(approval && approvals_break))
}

/// An agent's sound (at the system's sounds' volume), as the person's preferences allow.
pub fn play_agent(id: &str) {
    let prefs = crate::agent_prefs::load();
    let dnd = crate::shell_atual().is_some_and(|s| s.notificacoes().dnd_now());
    let approval = id == "kalango-agent-approval";
    if !agent_sound_allowed(prefs.sound, dnd, prefs.respect_dnd, approval, crate::notification_policy::read().breaks.approvals) {
        if crate::configuracoes::depurando() {
            eprintln!("debug som: {id} calado ({})", if prefs.sound { "não perturbe" } else { "som dos agentes desligado" });
        }
        return;
    }
    crate::configuracoes_som::tocar(id);
}

/// The sounds other KalangoOS programs may ask the shell for (the app action `play-sound`, over D-Bus): the approval
/// screen's decisions (an agent's sound: its preferences), and Kalango Remote's session starting and ending (the
/// Remote app asks: the shell does not see its sessions).
pub const REQUESTABLE: [&str; 4] = ["kalango-agent-approved", "kalango-agent-denied", "kalango-remote-start", "kalango-remote-stop"];

/// `play-sound` on the shell's application (`gdbus call … org.gtk.Actions.Activate play-sound [<'kalango-agent-approved'>] {}`).
pub fn export_action(app: &gtk::gio::Application) {
    use gtk::gio::prelude::*;
    let a = gtk::gio::SimpleAction::new("play-sound", Some(gtk::glib::VariantTy::STRING));
    a.connect_activate(|_, p| {
        let Some(id) = p.and_then(|p| p.get::<String>()) else { return };
        if id.starts_with("kalango-agent-") && REQUESTABLE.contains(&id.as_str()) {
            play_agent(&id);
        } else if REQUESTABLE.contains(&id.as_str()) {
            crate::configuracoes_som::tocar(&id);
        } else if crate::configuracoes::depurando() {
            eprintln!("debug som: pedido recusado: {id}");
        }
    });
    app.add_action(&a);
}

/// The sound when the session starts, from what was kept at the last start (the boot and the session it was in):
/// the first session since the computer was turned on is its start; another session is a login; the shell starting
/// again in the same session (an update, a crash) is silent.
pub fn session_start_sound(kept: &serde_json::Value, boot: &str, session: &str) -> Option<&'static str> {
    if kept["boot"].as_str() == Some(boot) && kept["session"].as_str() == Some(session) {
        return None;
    }
    Some(if kept["boot"].as_str() == Some(boot) { "desktop-login" } else { "system-bootup" })
}

fn session_state_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| gtk::glib::home_dir().join(".local/state")).join("kalango/sessao-som.json")
}

/// The shell is ready: the start's or the login's sound, once a session.
pub fn at_session_start() {
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap_or_default().trim().to_string();
    // (the compositor's socket names this session; the login's id where there is none)
    let session = std::env::var("NIRI_SOCKET").or_else(|_| std::env::var("XDG_SESSION_ID")).unwrap_or_default();
    let path = session_state_path();
    let kept: serde_json::Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let Some(id) = session_start_sound(&kept, &boot, &session) else { return };
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&path, serde_json::json!({"boot": boot, "session": session}).to_string());
    crate::configuracoes_som::tocar(id);
}

/// The charger's sound for a change of power source (none for the first reading).
pub fn charger_sound(before: Option<bool>, on_ac: bool) -> Option<&'static str> {
    match before {
        Some(b) if b != on_ac => Some(if on_ac { "power-plug" } else { "power-unplug" }),
        _ => None,
    }
}

thread_local! {
    static ON_AC: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// A reading of the power source (UPower's OnBattery signal; the session test's battery samples).
pub fn power_source_seen(on_ac: bool) {
    if let Some(id) = charger_sound(ON_AC.with(|c| c.replace(Some(on_ac))), on_ac) {
        crate::configuracoes_som::tocar(id);
    }
}

/// Follows UPower's OnBattery (a signal: nothing polls). The session test has no UPower: its battery samples tell.
pub fn watch_power_source() {
    if std::env::var_os("KALANGO_UPOWER").is_some() || std::env::var_os("KALANGO_SHELL_DEV").is_some() {
        return;
    }
    use gtk::gio;
    use gtk::gio::prelude::*;
    gio::DBusProxy::for_bus(gio::BusType::System, gio::DBusProxyFlags::NONE, None, "org.freedesktop.UPower", "/org/freedesktop/UPower",
        "org.freedesktop.UPower", gio::Cancellable::NONE, |p| {
            let Ok(p) = p else { return };
            let read = |p: &gio::DBusProxy| p.cached_property("OnBattery").and_then(|v| v.get::<bool>());
            if let Some(b) = read(&p) {
                power_source_seen(!b);
            }
            p.connect_local("g-properties-changed", false, move |args| {
                if let Some(p) = args.first().and_then(|a| a.get::<gio::DBusProxy>().ok()) {
                    if let Some(b) = read(&p) {
                        power_source_seen(!b);
                    }
                }
                None
            });
            // (kept for the session's life)
            std::mem::forget(p);
        });
}

/// How long the network has to stay as it is before its sound plays: the start, a Wi-Fi changing access point or a
/// cable wiggled go up and down in a few seconds.
pub const NETWORK_SETTLE: Duration = Duration::from_secs(4);
/// No network sound this long after the shell starts (the network coming up at login is not news).
pub const NETWORK_QUIET_AT_START: Duration = Duration::from_secs(30);

/// The network's sound for a state that held for NETWORK_SETTLE: none for the first one known, nor when it is the
/// one last said.
pub fn network_sound(said: Option<bool>, online: bool) -> Option<&'static str> {
    match said {
        Some(s) if s != online => Some(if online { "network-connectivity-established" } else { "network-connectivity-lost" }),
        _ => None,
    }
}

thread_local! {
    /// the state last said (or first seen), and the timer waiting for the state to settle
    static NETWORK: std::cell::RefCell<(Option<bool>, Option<gtk::glib::SourceId>)> = const { std::cell::RefCell::new((None, None)) };
}

/// Online as the person means it: the internet reachable (GIO's NetworkMonitor, NetworkManager's connectivity).
fn online(m: &gtk::gio::NetworkMonitor) -> bool {
    use gtk::gio::prelude::*;
    m.connectivity() == gtk::gio::NetworkConnectivity::Full
}

/// Follows the network (NetworkManager's signals through GIO's NetworkMonitor: nothing polls). A change says itself
/// once it held for NETWORK_SETTLE, and not in the first NETWORK_QUIET_AT_START.
pub fn watch_network() {
    use gtk::gio::prelude::*;
    use gtk::glib;
    // (the development session's network is the desktop around it; the session test has none of its own)
    if std::env::var_os("KALANGO_SHELL_DEV").is_some() {
        return;
    }
    let m = gtk::gio::NetworkMonitor::default();
    let started = std::time::Instant::now();
    NETWORK.with(|n| n.borrow_mut().0 = Some(online(&m)));
    let changed = move |m: &gtk::gio::NetworkMonitor| {
        let m = m.clone();
        NETWORK.with(|n| {
            if let Some(t) = n.borrow_mut().1.take() {
                t.remove();
            }
            let id = glib::timeout_add_local_once(NETWORK_SETTLE, move || {
                let now = online(&m);
                let said = NETWORK.with(|n| {
                    let mut n = n.borrow_mut();
                    n.1 = None;
                    n.0.replace(now)
                });
                if started.elapsed() < NETWORK_QUIET_AT_START {
                    return;
                }
                if crate::configuracoes::depurando() {
                    eprintln!("debug som: rede {}", if now { "conectada" } else { "perdida" });
                }
                if let Some(id) = network_sound(said, now) {
                    crate::configuracoes_som::tocar(id);
                }
            });
            n.borrow_mut().1 = Some(id);
        });
    };
    let c = changed.clone();
    m.connect_network_changed(move |m, _| c(m));
    m.connect_notify_local(Some("connectivity"), move |m, _| changed(m));
}

/// The update's sound: PackageKit has one prepared now, and had none (not one already there when the shell starts).
pub fn update_sound(before: Option<bool>, prepared: bool) -> Option<&'static str> {
    (before == Some(false) && prepared).then_some("kalango-update-ready")
}

/// Follows PackageKit's prepared update (its Offline.UpdatePrepared property, by its signal: nothing polls). Settings
/// › System and the power menu read it; this says when one becomes ready.
pub fn watch_update() {
    use gtk::gio;
    use gtk::gio::prelude::*;
    if std::env::var_os("KALANGO_SHELL_DEV").is_some() && std::env::var_os("KALANGO_PACKAGEKIT_BUS").is_none() {
        return;
    }
    gio::DBusProxy::for_bus(crate::energia::packagekit_bus(), gio::DBusProxyFlags::DO_NOT_AUTO_START, None, "org.freedesktop.PackageKit",
        "/org/freedesktop/PackageKit", "org.freedesktop.PackageKit.Offline", gio::Cancellable::NONE, |p| {
            let Ok(p) = p else { return };
            let read = |p: &gio::DBusProxy| p.cached_property("UpdatePrepared").and_then(|v| v.get::<bool>());
            let before = std::rc::Rc::new(std::cell::Cell::new(read(&p)));
            p.connect_local("g-properties-changed", false, move |args| {
                let p = args.first().and_then(|a| a.get::<gio::DBusProxy>().ok())?;
                let now = read(&p)?;
                if let Some(id) = update_sound(before.replace(Some(now)), now) {
                    crate::configuracoes_som::tocar(id);
                }
                None
            });
            std::mem::forget(p);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme_dir() -> PathBuf {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/sounds")).to_path_buf()
    }

    /// Every event ID the shell plays.
    const PLAYED: [&str; 35] = [
        "kalango-screen-lock", "kalango-screen-unlock", "kalango-auth-failed", "kalango-screen-record-start",
        "kalango-screen-record-stop", "alarm-clock-elapsed", "kalango-agent-start", "network-connectivity-established",
        "network-connectivity-lost", "kalango-update-ready",
        "system-bootup", "desktop-login", "desktop-logout", "system-shutdown",
        "message-new-instant", "dialog-warning", "dialog-error", "complete",
        "trash-empty", "device-added", "device-removed", "power-plug", "power-unplug", "battery-low", "screen-capture",
        "audio-volume-change",
        "kalango-agent-request", "kalango-agent-approval", "kalango-agent-approved", "kalango-agent-denied",
        "kalango-agent-done", "kalango-agent-failed", "kalango-undo", "kalango-voice-start", "kalango-voice-stop",
    ];

    #[test]
    fn the_theme_has_every_sound_the_shell_plays() {
        for id in PLAYED.iter().chain(&REQUESTABLE) {
            let p = theme_dir().join("kalango/stereo").join(format!("{id}.oga"));
            assert!(p.is_file(), "{id} falta no tema");
        }
    }

    #[test]
    fn agent_sounds_follow_the_preferences() {
        assert!(agent_sound_allowed(true, false, true, false, false));
        assert!(!agent_sound_allowed(false, false, true, true, true));
        // Do not disturb: quiet, unless the agents may pass it, or it is an approval and approvals pass
        assert!(!agent_sound_allowed(true, true, true, false, true));
        assert!(agent_sound_allowed(true, true, false, false, false));
        assert!(agent_sound_allowed(true, true, true, true, true));
        assert!(!agent_sound_allowed(true, true, true, true, false));
    }

    #[test]
    fn the_start_s_sound_once_a_session() {
        let none = serde_json::Value::Null;
        assert_eq!(session_start_sound(&none, "b1", "s1"), Some("system-bootup"));
        let kept = serde_json::json!({"boot": "b1", "session": "s1"});
        assert_eq!(session_start_sound(&kept, "b1", "s1"), None);
        assert_eq!(session_start_sound(&kept, "b1", "s2"), Some("desktop-login"));
        assert_eq!(session_start_sound(&kept, "b2", "s1"), Some("system-bootup"));
    }

    #[test]
    fn the_charger_s_sound_on_a_change_only() {
        assert_eq!(charger_sound(None, true), None);
        assert_eq!(charger_sound(Some(true), true), None);
        assert_eq!(charger_sound(Some(false), true), Some("power-plug"));
        assert_eq!(charger_sound(Some(true), false), Some("power-unplug"));
    }

    #[test]
    fn the_network_says_a_change_once() {
        assert_eq!(network_sound(None, true), None);
        assert_eq!(network_sound(Some(true), true), None);
        assert_eq!(network_sound(Some(true), false), Some("network-connectivity-lost"));
        assert_eq!(network_sound(Some(false), true), Some("network-connectivity-established"));
        // (the settling and the quiet start are longer than a boot's flaps and the login's sound)
        assert!(NETWORK_SETTLE >= Duration::from_secs(3) && NETWORK_QUIET_AT_START >= Duration::from_secs(20));
    }

    #[test]
    fn an_update_says_itself_when_it_becomes_ready() {
        assert_eq!(update_sound(None, true), None);
        assert_eq!(update_sound(Some(true), true), None);
        assert_eq!(update_sound(Some(false), false), None);
        assert_eq!(update_sound(Some(true), false), None);
        assert_eq!(update_sound(Some(false), true), Some("kalango-update-ready"));
    }

    /// For measuring (not part of the suite): plays `KALANGO_SONS_MEDIR_N` sounds through `play` (as the shell does)
    /// into `KALANGO_SONS_TESTE_DESTINO`, printing the wall clock (ms) at each call, 1.6 s apart.
    #[test]
    #[ignore]
    fn measure_plays() {
        let n: usize = std::env::var("KALANGO_SONS_MEDIR_N").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        if let Some(dest) = std::env::var("KALANGO_SONS_TESTE_DESTINO").ok().filter(|d| !d.is_empty()) {
            std::env::set_var("KALANGO_SONS_DESTINO", dest);
        }
        for _ in 0..n {
            let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64() * 1000.0;
            println!("T {ms:.1}");
            let id = std::env::var("KALANGO_SONS_MEDIR_ID").unwrap_or_else(|_| "message-new-instant".into());
            let v = std::env::var("KALANGO_SONS_MEDIR_VOLUME").ok().and_then(|v| v.parse().ok()).unwrap_or(0.4);
            assert!(play(&id, v).is_some());
            std::thread::sleep(Duration::from_millis(1600));
        }
    }
}
