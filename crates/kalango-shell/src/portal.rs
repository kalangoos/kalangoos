//! KalangoOS's portal backend (org.freedesktop.impl.portal.desktop.kalango): the FileChooser that every app's
//! "Abrir" and "Salvar" goes through (designs "Abrir arquivo" and "Salvar como", the chooser in arquivos_seletor.rs),
//! Access, the permission request of the Camera, Device and Location portals (design "Permissão de app",
//! access_dialog.rs), and ScreenCast, the system's choice of what an app sees (design "Compartilhar tela",
//! screen_share.rs).
//! xdg-desktop-portal picks it for the KalangoOS desktop (kalangoos-portals.conf); the other portals stay with the
//! GTK backend. Apps reach it as the session sets them to (GDK_DEBUG=portals for GTK 4, GTK_USE_PORTAL=1 for GTK 3).

use crate::arquivos::seletor::{self, Filtro, Modo, Pedido};
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use kalango_i18n::tr;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

const NAME: &str = "org.freedesktop.impl.portal.desktop.kalango";
const PATH: &str = "/org/freedesktop/portal/desktop";

const XML: &str = r#"<node>
  <interface name="org.freedesktop.impl.portal.Settings">
    <method name="ReadAll">
      <arg type="as" name="namespaces" direction="in"/>
      <arg type="a{sa{sv}}" name="value" direction="out"/>
    </method>
    <method name="Read">
      <arg type="s" name="namespace" direction="in"/>
      <arg type="s" name="key" direction="in"/>
      <arg type="v" name="value" direction="out"/>
    </method>
    <signal name="SettingChanged">
      <arg type="s" name="namespace"/>
      <arg type="s" name="key"/>
      <arg type="v" name="value"/>
    </signal>
    <property name="version" type="u" access="read"/>
  </interface>
  <interface name="org.freedesktop.impl.portal.FileChooser">
    <method name="OpenFile">
      <arg type="o" name="handle" direction="in"/><arg type="s" name="app_id" direction="in"/>
      <arg type="s" name="parent_window" direction="in"/><arg type="s" name="title" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/><arg type="a{sv}" name="results" direction="out"/>
    </method>
    <method name="SaveFile">
      <arg type="o" name="handle" direction="in"/><arg type="s" name="app_id" direction="in"/>
      <arg type="s" name="parent_window" direction="in"/><arg type="s" name="title" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/><arg type="a{sv}" name="results" direction="out"/>
    </method>
    <method name="SaveFiles">
      <arg type="o" name="handle" direction="in"/><arg type="s" name="app_id" direction="in"/>
      <arg type="s" name="parent_window" direction="in"/><arg type="s" name="title" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/><arg type="a{sv}" name="results" direction="out"/>
    </method>
  </interface>
  <interface name="org.freedesktop.impl.portal.ScreenCast">
    <method name="CreateSession">
      <arg type="o" name="handle" direction="in"/><arg type="o" name="session_handle" direction="in"/>
      <arg type="s" name="app_id" direction="in"/><arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/><arg type="a{sv}" name="results" direction="out"/>
    </method>
    <method name="SelectSources">
      <arg type="o" name="handle" direction="in"/><arg type="o" name="session_handle" direction="in"/>
      <arg type="s" name="app_id" direction="in"/><arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/><arg type="a{sv}" name="results" direction="out"/>
    </method>
    <method name="Start">
      <arg type="o" name="handle" direction="in"/><arg type="o" name="session_handle" direction="in"/>
      <arg type="s" name="app_id" direction="in"/><arg type="s" name="parent_window" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/><arg type="a{sv}" name="results" direction="out"/>
    </method>
    <property name="AvailableSourceTypes" type="u" access="read"/>
    <property name="AvailableCursorModes" type="u" access="read"/>
    <property name="version" type="u" access="read"/>
  </interface>
  <interface name="org.freedesktop.impl.portal.Print">
    <method name="PreparePrint">
      <arg type="o" name="handle" direction="in"/><arg type="s" name="app_id" direction="in"/><arg type="s" name="parent_window" direction="in"/>
      <arg type="s" name="title" direction="in"/><arg type="a{sv}" name="settings" direction="in"/><arg type="a{sv}" name="page_setup" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/><arg type="u" name="response" direction="out"/><arg type="a{sv}" name="results" direction="out"/>
    </method>
    <method name="Print">
      <arg type="o" name="handle" direction="in"/><arg type="s" name="app_id" direction="in"/><arg type="s" name="parent_window" direction="in"/>
      <arg type="s" name="title" direction="in"/><arg type="h" name="fd" direction="in"/><arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/><arg type="a{sv}" name="results" direction="out"/>
    </method>
    <property name="version" type="u" access="read"/>
  </interface>
  <interface name="org.freedesktop.impl.portal.Access">
    <method name="AccessDialog">
      <arg type="o" name="handle" direction="in"/><arg type="s" name="app_id" direction="in"/>
      <arg type="s" name="parent_window" direction="in"/><arg type="s" name="title" direction="in"/>
      <arg type="s" name="subtitle" direction="in"/><arg type="s" name="body" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/><arg type="a{sv}" name="results" direction="out"/>
    </method>
  </interface>
</node>"#;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// The app as the person knows it: its desktop file's name; an app outside a sandbox often comes without an id,
/// and then it is the window in focus (the one whose button asked).
fn app_name(app_id: &str) -> String {
    // with an id: its desktop file's name, or the id's last part ("com.discordapp.Discord" → "Discord")
    if !app_id.is_empty() {
        return gio::DesktopAppInfo::new(&format!("{app_id}.desktop")).map(|a| a.name().to_string())
            .unwrap_or_else(|| app_id.rsplit('.').next().unwrap_or(app_id).to_string());
    }
    // without one: the window in focus
    let Some(s) = crate::shell_atual() else { return String::new() };
    let Some(j) = s.estado.janelas.borrow().values().find(|j| j.focada).cloned() else { return String::new() };
    gio::DesktopAppInfo::new(&format!("{}.desktop", j.app_id)).map(|a| a.name().to_string()).unwrap_or(j.titulo)
}

/// A path the portal sends as bytes (NUL-terminated).
fn path_of(v: Option<glib::Variant>) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let b: Vec<u8> = v?.get()?;
    let b: Vec<u8> = b.into_iter().take_while(|c| *c != 0).collect();
    (!b.is_empty()).then(|| PathBuf::from(std::ffi::OsStr::from_bytes(&b)))
}

/// The app's filters: (name, [(0 glob | 1 mime, pattern)]).
fn filters_of(v: Option<glib::Variant>) -> Vec<Filtro> {
    let list: Vec<(String, Vec<(u32, String)>)> = v.and_then(|v| v.get()).unwrap_or_default();
    list.into_iter().map(|(nome, regras)| {
        let mut f = Filtro { nome, ..Default::default() };
        for (k, p) in regras {
            if k == 0 { f.globs.push(p) } else { f.mimes.push(p) }
        }
        f
    }).collect()
}

fn filter_variant(f: &Filtro) -> glib::Variant {
    let regras: Vec<(u32, String)> = f.globs.iter().map(|g| (0u32, g.clone())).chain(f.mimes.iter().map(|m| (1u32, m.clone()))).collect();
    (f.nome.clone(), regras).to_variant()
}

/// (ua{sv}): built from its parts — a tuple holding a Variant would box it as "v".
fn reply(inv: gio::DBusMethodInvocation, response: u32, results: glib::VariantDict) {
    inv.return_value(Some(&glib::Variant::tuple_from_iter([response.to_variant(), results.end()])));
}

fn choose(method: &str, params: &glib::Variant, inv: gio::DBusMethodInvocation, c: &gio::DBusConnection) {
    let handle: String = params.child_value(0).get::<glib::variant::ObjectPath>().map(|p| p.as_str().to_string()).unwrap_or_default();
    let app_id: String = params.child_value(1).get().unwrap_or_default();
    let title: String = params.child_value(3).get().unwrap_or_default();
    let opts = glib::VariantDict::new(Some(&params.child_value(4)));
    let get_b = |k: &str| opts.lookup_value(k, None).and_then(|v| v.get::<bool>()).unwrap_or(false);
    let filtros = filters_of(opts.lookup_value("filters", None));
    let atual: Option<(String, Vec<(u32, String)>)> = opts.lookup_value("current_filter", None).and_then(|v| v.get());
    let filtro = atual.and_then(|(n, _)| filtros.iter().position(|f| f.nome == n)).or((!filtros.is_empty()).then_some(0));
    let salvar = method != "OpenFile";
    let current_file = path_of(opts.lookup_value("current_file", None));
    let pasta = path_of(opts.lookup_value("current_folder", None)).or_else(|| current_file.as_ref().and_then(|p| p.parent().map(|x| x.to_path_buf())));
    let nome = opts.lookup_value("current_name", None).and_then(|v| v.get::<String>())
        .or_else(|| current_file.as_ref().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())))
        .unwrap_or_else(|| tr("Untitled").to_string());
    let modo = if salvar { Modo::Salvar { nome } } else { Modo::Abrir { varios: get_b("multiple"), pasta: get_b("directory") } };
    let titulo = if !title.is_empty() { title } else if salvar { tr("Save as").to_string() } else { tr("Open file").to_string() };
    let rotulo = opts.lookup_value("accept_label", None).and_then(|v| v.get::<String>());
    let app = app_name(&app_id);
    if debug() {
        eprintln!("debug portal: {method} de «{app}» ({} filtros)", filtros.len());
    }
    let filtros_resposta = filtros.clone();
    let pedido = Pedido { modo, titulo, app, rotulo, filtros, filtro, pasta };
    // the portal's request object: Close() cancels the chooser
    let janela = seletor::escolher(pedido, move |r| {
        let d = glib::VariantDict::new(None);
        match r {
            Some((paths, f)) => {
                let uris: Vec<String> = paths.iter().map(|p| gio::File::for_path(p).uri().to_string()).collect();
                d.insert_value("uris", &uris.to_variant());
                if let Some(f) = f.and_then(|i| filtros_resposta.get(i)) {
                    d.insert_value("current_filter", &filter_variant(f));
                }
                reply(inv, 0, d);
            }
            None => reply(inv, 1, d),
        }
    });
    if let (Some(j), Ok(info)) = (janela, gio::DBusNodeInfo::for_xml(r#"<node><interface name="org.freedesktop.impl.portal.Request"><method name="Close"/></interface></node>"#)) {
        if let Some(iface) = info.lookup_interface("org.freedesktop.impl.portal.Request") {
            let jw = j.downgrade();
            let id = c.register_object(&handle, &iface).method_call(move |_, _, _, _, _, _, inv| {
                if let Some(j) = jw.upgrade() {
                    j.close();
                }
                inv.return_value(None);
            }).build();
            if let Ok(id) = id {
                let c2 = c.clone();
                let id = std::cell::RefCell::new(Some(id));
                j.connect_destroy(move |_| {
                    if let Some(id) = id.borrow_mut().take() {
                        let _ = c2.unregister_object(id);
                    }
                });
            }
        }
    }
}

/// The portal's request object for one dialog: Close() calls `close` (the dialog then answers "cancelled"); the object
/// goes away with the reply.
fn request_object(c: &gio::DBusConnection, handle: &str, close: impl Fn() + 'static) -> Option<gio::RegistrationId> {
    let info = gio::DBusNodeInfo::for_xml(r#"<node><interface name="org.freedesktop.impl.portal.Request"><method name="Close"/></interface></node>"#).ok()?;
    let iface = info.lookup_interface("org.freedesktop.impl.portal.Request")?;
    c.register_object(handle, &iface).method_call(move |_, _, _, _, _, _, inv| {
        close();
        inv.return_value(None);
    }).build().ok()
}

/// Print, its first half: the app's settings taken as they are (the choices are made in the dialog, which shows the
/// real pages: it opens when the document comes, in `print`), and a token for the second half.
fn prepare_print(params: &glib::Variant, inv: gio::DBusMethodInvocation) {
    let app: String = params.child_value(1).get().unwrap_or_default();
    if debug() {
        eprintln!("debug portal: PreparePrint de «{}»", app_name(&app));
    }
    let d = glib::VariantDict::new(None);
    d.insert_value("settings", &params.child_value(4));
    d.insert_value("page-setup", &params.child_value(5));
    d.insert_value("token", &1u32.to_variant());
    reply(inv, 0, d);
}

/// Print, its second half: the document (a PDF the app made, by its file descriptor) into the system's dialog.
fn print(params: &glib::Variant, inv: gio::DBusMethodInvocation, c: &gio::DBusConnection) {
    use std::io::{Read, Write};
    use std::os::fd::FromRawFd;
    let handle: String = params.child_value(0).get::<glib::variant::ObjectPath>().map(|p| p.as_str().to_string()).unwrap_or_default();
    let app_id: String = params.child_value(1).get().unwrap_or_default();
    let title: String = params.child_value(3).get().unwrap_or_default();
    let idx = params.child_value(4).get::<glib::variant::Handle>().map(|h| h.0).unwrap_or(0);
    let fd = inv.message().unix_fd_list().and_then(|l| l.get(idx).ok());
    let Some(fd) = fd else {
        return reply(inv, 2, glib::VariantDict::new(None));
    };
    let mut src = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut bytes = Vec::new();
    let _ = src.read_to_end(&mut bytes);
    let dir = glib::user_runtime_dir().join("kalango-imprimir");
    let _ = std::fs::create_dir_all(&dir);
    let pdf = dir.join(format!("{}-{}.pdf", std::process::id(), glib::monotonic_time()));
    if std::fs::File::create(&pdf).and_then(|mut f| f.write_all(&bytes)).is_err() {
        return reply(inv, 2, glib::VariantDict::new(None));
    }
    if debug() {
        eprintln!("debug portal: Print de «{}» ({} bytes)", app_name(&app_id), bytes.len());
    }
    let title = if title.is_empty() { tr("Document").to_string() } else { title };
    let reg = std::rc::Rc::new(std::cell::RefCell::new(None::<gio::RegistrationId>));
    let (c2, reg2, pdf2) = (c.clone(), reg.clone(), pdf.clone());
    crate::print_dialog::ask(pdf, title, app_name(&app_id), move |o| {
        if let Some(r) = reg2.borrow_mut().take() {
            let _ = c2.unregister_object(r);
        }
        let _ = std::fs::remove_file(&pdf2);
        if let (crate::print_dialog::Outcome::Saved(f), true) = (&o, debug()) {
            eprintln!("debug portal: Print salvo em «{}»", f.display());
        }
        reply(inv, if matches!(o, crate::print_dialog::Outcome::Cancelled) { 1 } else { 0 }, glib::VariantDict::new(None));
    });
    *reg.borrow_mut() = request_object(c, &handle, || {});
}

/// Access: a permission request (camera, microphone, speakers, location) in the system's dialog.
fn access(params: &glib::Variant, inv: gio::DBusMethodInvocation, c: &gio::DBusConnection) {
    use crate::access_dialog::{self, Choice, Request, Sensor};
    let handle: String = params.child_value(0).get::<glib::variant::ObjectPath>().map(|p| p.as_str().to_string()).unwrap_or_default();
    let app_id: String = params.child_value(1).get().unwrap_or_default();
    let title: String = params.child_value(3).get().unwrap_or_default();
    let subtitle: String = params.child_value(4).get().unwrap_or_default();
    let body: String = params.child_value(5).get().unwrap_or_default();
    let opts = glib::VariantDict::new(Some(&params.child_value(6)));
    let icon = opts.lookup_value("icon", None).and_then(|v| v.get::<String>()).unwrap_or_default();
    let sensor = Sensor::from_icon(&icon);
    let app = app_name(&app_id);
    if debug() {
        eprintln!("debug portal: AccessDialog de «{app}» ({icon})");
    }
    // the dialog has its own words for the sensors it knows; for anything else, the portal's
    let reason = if sensor.is_some() { String::new() } else { [subtitle, body].iter().filter(|t| !t.is_empty()).cloned().collect::<Vec<_>>().join(" ") };
    let reg = std::rc::Rc::new(std::cell::RefCell::new(request_object(c, &handle, access_dialog::withdraw)));
    let (c2, reg2, id2) = (c.clone(), reg.clone(), app_id.clone());
    access_dialog::ask(Request { app, app_id, sensor, title, reason }, move |choice| {
        if let Some(r) = reg2.borrow_mut().take() {
            let _ = c2.unregister_object(r);
        }
        if let (Choice::Once, Some(s)) = (choice, sensor) {
            access_dialog::take_back_when_closed(id2, s);
        }
        reply(inv, if choice == Choice::Deny { 1 } else { 0 }, glib::VariantDict::new(None));
    });
}

/// ScreenCast: the app's session, what it accepts, and the system's chooser at Start.
fn screencast(method: &str, params: &glib::Variant, inv: gio::DBusMethodInvocation, c: &gio::DBusConnection) {
    use crate::screen_share;
    let path = |i: usize| params.child_value(i).get::<glib::variant::ObjectPath>().map(|p| p.as_str().to_string()).unwrap_or_default();
    let (handle, session) = (path(0), path(1));
    let app_id: String = params.child_value(2).get().unwrap_or_default();
    match method {
        "CreateSession" => {
            screen_share::create_session(c, &session, &app_name(&app_id));
            reply(inv, 0, glib::VariantDict::new(None));
        }
        "SelectSources" => {
            let opts = glib::VariantDict::new(Some(&params.child_value(3)));
            let types = opts.lookup_value("types", None).and_then(|v| v.get::<u32>()).unwrap_or(screen_share::SOURCE_MONITOR);
            let cursor = opts.lookup_value("cursor_mode", None).and_then(|v| v.get::<u32>()).unwrap_or(1);
            screen_share::select_sources(&session, types, cursor);
            reply(inv, 0, glib::VariantDict::new(None));
        }
        _ => {
            if debug() {
                eprintln!("debug portal: ScreenCast.Start de «{}»", app_name(&app_id));
            }
            let reg = std::rc::Rc::new(std::cell::RefCell::new(request_object(c, &handle, screen_share::withdraw)));
            let (c2, reg2) = (c.clone(), reg.clone());
            screen_share::start(c, &session, &app_id, move |response, results| {
                if let Some(r) = reg2.borrow_mut().take() {
                    let _ = c2.unregister_object(r);
                }
                inv.return_value(Some(&glib::Variant::tuple_from_iter([response.to_variant(), results])));
            });
        }
    }
}

/// The session's portal environment: xdg-desktop-portal reads the desktop's name (and, in a session run from the
/// repository, where KalangoOS's portal files are) from the bus's activation environment.
fn activation_environment(c: &gio::DBusConnection) {
    let mut env: Vec<(String, String)> = vec![("XDG_CURRENT_DESKTOP".into(), "KalangoOS".into())];
    for k in ["XDG_DESKTOP_PORTAL_DIR", "XDG_CONFIG_DIRS", "GDK_DEBUG", "GTK_USE_PORTAL"] {
        if let Ok(v) = std::env::var(k) {
            env.push((k.into(), v));
        }
    }
    // a session run from the repository: KalangoOS's portal files are in its sessao/dist (among the data dirs), not
    // in /usr — xdg-desktop-portal is told where
    if std::env::var_os("XDG_DESKTOP_PORTAL_DIR").is_none() {
        let dados = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
        if let Some(d) = dados.split(':').map(std::path::Path::new).find(|d| !d.starts_with("/usr") && d.join("xdg-desktop-portal/portals/kalango.portal").is_file()) {
            env.push(("XDG_DESKTOP_PORTAL_DIR".into(), d.join("xdg-desktop-portal/portals").display().to_string()));
            if let Some(etc) = d.parent().map(|x| x.join("etc/xdg")).filter(|x| x.is_dir()) {
                let resto = std::env::var("XDG_CONFIG_DIRS").unwrap_or_else(|_| "/etc/xdg".into());
                env.retain(|(k, _)| k != "XDG_CONFIG_DIRS");
                env.push(("XDG_CONFIG_DIRS".into(), format!("{}:{resto}", etc.display())));
            }
        }
    }
    let map: std::collections::HashMap<String, String> = env.into_iter().collect();
    c.call(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", "org.freedesktop.DBus", "UpdateActivationEnvironment", Some(&(map,).to_variant()),
        None, gio::DBusCallFlags::NONE, 3000, gio::Cancellable::NONE, |_| {});
}

/// Owns the backend's name and serves the FileChooser and Access.
// ------------------------------------------------------------------------------------------------------ Settings

/// The appearance apps follow (GTK 4 and libadwaita ask the portal): dark or light as the shell is now, its accent,
/// normal contrast. So the apps follow KalangoOS's theme, also when it changes, without writing GNOME's settings.
fn appearance() -> Vec<(&'static str, glib::Variant)> {
    let t = crate::tema::atual();
    let hex = crate::tema::DESTAQUES.iter().find(|(id, _, _)| *id == t.destaque).map(|(_, h, _)| *h).unwrap_or(crate::tema::ACENTO);
    let (r, g, b) = crate::tema::rgb(&crate::tema::cor(hex));
    vec![
        // 1: prefer dark, 2: prefer light
        ("color-scheme", (if crate::tema::e_claro() { 2u32 } else { 1u32 }).to_variant()),
        ("accent-color", (r, g, b).to_variant()),
        ("contrast", 0u32.to_variant()),
    ]
}

const APPEARANCE: &str = "org.freedesktop.appearance";
const INTERFACE: &str = "org.gnome.desktop.interface";

/// What GTK 4 reads of GNOME's interface settings (through the portal, on Wayland): KalangoOS's — not the theme, icons,
/// cursor and font of whatever system the session runs on.
fn interface() -> Vec<(&'static str, glib::Variant)> {
    let claro = crate::tema::e_claro();
    let cursor = std::env::var("XCURSOR_THEME").ok().filter(|c| !c.is_empty()).unwrap_or_else(|| "Adwaita".into());
    let tamanho = std::env::var("XCURSOR_SIZE").ok().and_then(|s| s.parse::<i32>().ok()).unwrap_or(24);
    vec![
        ("gtk-theme", (if claro { "Adwaita" } else { "Adwaita-dark" }).to_variant()),
        ("icon-theme", "Adwaita".to_variant()),
        ("cursor-theme", cursor.to_variant()),
        ("cursor-size", tamanho.to_variant()),
        ("font-name", format!("{} 11", crate::tema::fonte_ui()).to_variant()),
        ("color-scheme", (if claro { "prefer-light" } else { "prefer-dark" }).to_variant()),
    ]
}

fn namespace(ns: &str) -> Option<Vec<(&'static str, glib::Variant)>> {
    match ns {
        APPEARANCE => Some(appearance()),
        INTERFACE => Some(interface()),
        _ => None,
    }
}

/// Whether a ReadAll pattern ("org.freedesktop.appearance", "org.freedesktop.*", or none at all) covers the namespace.
fn covers(patterns: &[String], ns: &str) -> bool {
    patterns.is_empty() || patterns.iter().any(|p| p.is_empty() || p == ns || p.strip_suffix('*').is_some_and(|pre| ns.starts_with(pre)))
}

fn settings(method: &str, params: &glib::Variant, inv: gio::DBusMethodInvocation) {
    match method {
        "ReadAll" => {
            let patterns: Vec<String> = params.child_value(0).get().unwrap_or_default();
            let mut all: HashMap<String, HashMap<String, glib::Variant>> = HashMap::new();
            for ns in [APPEARANCE, INTERFACE] {
                if covers(&patterns, ns) {
                    all.insert(ns.into(), namespace(ns).unwrap_or_default().into_iter().map(|(k, v)| (k.to_string(), v)).collect());
                }
            }
            inv.return_value(Some(&(all,).to_variant()));
        }
        _ => {
            let (ns, key): (String, String) = params.get().unwrap_or_default();
            match namespace(&ns).unwrap_or_default().into_iter().find(|(k, _)| *k == key) {
                Some((_, v)) => inv.return_value(Some(&(v,).to_variant())),
                None => inv.return_dbus_error("org.freedesktop.portal.Error.NotFound", "no such setting"),
            }
        }
    }
}

thread_local! {
    static CONEXAO: RefCell<Option<gio::DBusConnection>> = const { RefCell::new(None) };
    /// what the apps were last told (only what changes goes out)
    static DITO: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

/// The theme changed: the apps are told (SettingChanged) what changed of the appearance.
fn appearance_changed() {
    let Some(c) = CONEXAO.with(|c| c.borrow().clone()) else { return };
    for (ns, k, v) in appearance().into_iter().map(|(k, v)| (APPEARANCE, k, v)).chain(interface().into_iter().map(|(k, v)| (INTERFACE, k, v))) {
        let k_full = format!("{ns} {k}");
        let texto = v.print(false).to_string();
        let mudou = DITO.with(|d| {
            let mut d = d.borrow_mut();
            match d.iter_mut().find(|(x, _)| *x == k_full) {
                Some(e) if e.1 == texto => false,
                Some(e) => {
                    e.1 = texto.clone();
                    true
                }
                None => {
                    d.push((k_full.clone(), texto.clone()));
                    true
                }
            }
        });
        if mudou {
            let _ = c.emit_signal(None, PATH, "org.freedesktop.impl.portal.Settings", "SettingChanged", Some(&(ns, k, v).to_variant()));
            if debug() {
                eprintln!("debug portal: {} {k} = {texto}", if ns == APPEARANCE { "aparência" } else { "interface" });
            }
        }
    }
}

pub fn start() {
    let Ok(c) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return };
    activation_environment(&c);
    let info = gio::DBusNodeInfo::for_xml(XML).expect("portal XML");
    for nome in ["org.freedesktop.impl.portal.FileChooser", "org.freedesktop.impl.portal.Access", "org.freedesktop.impl.portal.ScreenCast",
        "org.freedesktop.impl.portal.Settings", "org.freedesktop.impl.portal.Print"] {
        let iface = info.lookup_interface(nome).expect("portal interface");
        let c2 = c.clone();
        let r = c.register_object(PATH, &iface).method_call(move |_, _, _, _, method, params, inv| match method {
            "OpenFile" | "SaveFile" | "SaveFiles" => choose(method, &params, inv, &c2),
            "AccessDialog" => access(&params, inv, &c2),
            "CreateSession" | "SelectSources" | "Start" => screencast(method, &params, inv, &c2),
            "ReadAll" | "Read" => settings(method, &params, inv),
            "PreparePrint" => prepare_print(&params, inv),
            "Print" => print(&params, inv, &c2),
            _ => inv.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method),
        }).property(|_, _, _, iface, prop| match (iface, prop) {
            // a monitor or a window (no region: niri records outputs and windows); the cursor hidden or in the picture
            (_, "AvailableSourceTypes") => 3u32.to_variant(),
            (_, "AvailableCursorModes") => 3u32.to_variant(),
            ("org.freedesktop.impl.portal.ScreenCast", "version") => 5u32.to_variant(),
            _ => 1u32.to_variant(),
        }).build();
        if let Err(e) = r {
            eprintln!("kalango-shell: portal: {e}");
            return;
        }
    }
    let id = gio::bus_own_name_on_connection(&c, NAME, gio::BusNameOwnerFlags::REPLACE | gio::BusNameOwnerFlags::ALLOW_REPLACEMENT,
        |_, _| {
            if debug() {
                eprintln!("debug portal: seletor de arquivos ativo");
            }
        },
        |_, _| eprintln!("kalango-shell: portal: outro programa tem {NAME}"));
    std::mem::forget(id);
    CONEXAO.with(|x| *x.borrow_mut() = Some(c));
    appearance_changed();
    crate::tema::ao_mudar(appearance_changed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reply_is_ua_sv() {
        let d = glib::VariantDict::new(None);
        d.insert_value("uris", &vec!["file:///tmp/a"].to_variant());
        let v = glib::Variant::tuple_from_iter([0u32.to_variant(), d.end()]);
        assert_eq!(v.type_().as_str(), "(ua{sv})");
    }
}
