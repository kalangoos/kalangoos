//! The shell as NetworkManager's secret agent (org.freedesktop.NetworkManager.SecretAgent), as GNOME Shell is: when
//! a connection needs a secret — a Wi-Fi network joined from anywhere, a saved one whose password changed, an
//! enterprise network's user name and password — NetworkManager asks here and the password dialog
//! (wifi_password.rs) answers. NetworkManager keeps what it gets with the connection.
//!
//! Not in the development session, which runs inside someone's desktop: there the system's NetworkManager belongs to
//! that desktop's own agent. The session test uses a stand-in on the session bus (KALANGO_NM_BUS=session).

use crate::wifi_password::{self, Request};
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use kalango_i18n::tr;
use std::cell::RefCell;
use std::collections::HashMap;

const XML: &str = r#"<node>
  <interface name="org.freedesktop.NetworkManager.SecretAgent">
    <method name="GetSecrets">
      <arg name="connection" type="a{sa{sv}}" direction="in"/>
      <arg name="connection_path" type="o" direction="in"/>
      <arg name="setting_name" type="s" direction="in"/>
      <arg name="hints" type="as" direction="in"/>
      <arg name="flags" type="u" direction="in"/>
      <arg name="secrets" type="a{sa{sv}}" direction="out"/>
    </method>
    <method name="CancelGetSecrets">
      <arg name="connection_path" type="o" direction="in"/>
      <arg name="setting_name" type="s" direction="in"/>
    </method>
    <method name="SaveSecrets">
      <arg name="connection" type="a{sa{sv}}" direction="in"/>
      <arg name="connection_path" type="o" direction="in"/>
    </method>
    <method name="DeleteSecrets">
      <arg name="connection" type="a{sa{sv}}" direction="in"/>
      <arg name="connection_path" type="o" direction="in"/>
    </method>
  </interface>
</node>"#;

thread_local! {
    /// the connection NetworkManager is asking about, and its network: a cancel names the connection
    static PENDING: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

const ALLOW_INTERACTION: u32 = 0x1;
const REQUEST_NEW: u32 = 0x2;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

fn bus_type() -> Option<gio::BusType> {
    match std::env::var("KALANGO_NM_BUS").as_deref() {
        Ok("session") => Some(gio::BusType::Session),
        _ if std::env::var_os("KALANGO_SHELL_DEV").is_some() => None,
        _ => Some(gio::BusType::System),
    }
}

/// A connection's settings, as NetworkManager sends them: setting → key → value.
type Settings = HashMap<String, HashMap<String, glib::Variant>>;

/// What the person knows the network by, and its security as they know it.
fn describe(c: &Settings) -> (String, String, bool) {
    let get = |s: &str, k: &str| c.get(s).and_then(|m| m.get(k));
    let ssid = get("802-11-wireless", "ssid")
        .and_then(|v| v.get::<Vec<u8>>())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .or_else(|| get("connection", "id").and_then(|v| v.get::<String>()))
        .unwrap_or_default();
    let key_mgmt = get("802-11-wireless-security", "key-mgmt").and_then(|v| v.get::<String>()).unwrap_or_default();
    let enterprise = key_mgmt == "wpa-eap" || key_mgmt == "wpa-eap-suite-b-192" || c.contains_key("802-1x");
    let security = match key_mgmt.as_str() {
        "sae" => "WPA3",
        "wpa-psk" => "WPA2",
        "none" | "ieee8021x" => "WEP",
        "owe" => "OWE",
        _ if enterprise => "WPA2 Enterprise",
        _ => "",
    };
    (ssid, security.to_string(), enterprise)
}

fn uuid_of(c: &Settings) -> Option<String> {
    c.get("connection").and_then(|m| m.get("uuid")).and_then(|v| v.get::<String>())
}

/// The secrets NetworkManager asked for, in its shape: (a{sa{sv}}).
fn reply(setting: &str, security: &str, identity: Option<String>, password: String) -> glib::Variant {
    let mut inner: HashMap<String, glib::Variant> = HashMap::new();
    match setting {
        "802-1x" => {
            if let Some(i) = identity {
                inner.insert("identity".into(), i.to_variant());
            }
            inner.insert("password".into(), password.to_variant());
        }
        _ if security == "WEP" => {
            inner.insert("wep-key0".into(), password.to_variant());
        }
        _ => {
            inner.insert("psk".into(), password.to_variant());
        }
    }
    let outer: HashMap<String, HashMap<String, glib::Variant>> = HashMap::from([(setting.to_string(), inner)]);
    (outer,).to_variant()
}

/// Settings the person chose in the dialog that NetworkManager does not ask: "connect automatically" and the agents.
fn apply_choices(uuid: Option<String>, ssid: String, autoconnect: bool, agents: bool) {
    std::thread::spawn(move || {
        if !autoconnect {
            if let Some(u) = &uuid {
                let _ = std::process::Command::new(crate::configuracoes_rede::nmcli()).args(["connection", "modify", "uuid", u, "connection.autoconnect", "no"]).status();
            }
        }
        if !agents {
            let _ = crate::servicos::kalangod_rpc("kalango/redes_agentes", serde_json::json!({"rede": ssid, "permitir": false}));
        }
    });
}

fn get_secrets(params: &glib::Variant, inv: gio::DBusMethodInvocation) {
    let conn: Settings = params.child_value(0).get().unwrap_or_default();
    let path: String = params.child_value(1).get::<glib::variant::ObjectPath>().map(|p| p.as_str().to_string()).unwrap_or_default();
    let setting: String = params.child_value(2).get().unwrap_or_default();
    let flags: u32 = params.child_value(4).get().unwrap_or(0);
    let (ssid, security, enterprise) = describe(&conn);
    if debug() {
        eprintln!("debug wifi: o NetworkManager pede {setting} de {ssid} (flags {flags})");
    }
    // only what the dialog knows how to ask: a Wi-Fi password, an enterprise user name and password
    let wifi = conn.contains_key("802-11-wireless");
    if !(wifi && (setting == "802-11-wireless-security" || setting == "802-1x")) {
        inv.return_dbus_error("org.freedesktop.NetworkManager.SecretAgent.NoSecrets", "this agent only asks for Wi-Fi passwords");
        return;
    }
    if flags & ALLOW_INTERACTION == 0 {
        inv.return_dbus_error("org.freedesktop.NetworkManager.SecretAgent.NoSecrets", "the person was not to be asked");
        return;
    }
    let uuid = uuid_of(&conn);
    PENDING.with(|p| *p.borrow_mut() = Some((path, ssid.clone())));
    let req = Request { ssid: ssid.clone(), security: security.clone(), enterprise: enterprise || setting == "802-1x", wrong: flags & REQUEST_NEW != 0 };
    wifi_password::ask(req, move |answer| {
        PENDING.with(|p| p.borrow_mut().take());
        match answer {
        Some(a) => {
            if debug() {
                eprintln!("debug wifi: senha entregue ao NetworkManager ({ssid})");
            }
            apply_choices(uuid, ssid, a.autoconnect, a.agents);
            inv.return_value(Some(&reply(&setting, &security, a.identity, a.password)));
        }
        None => inv.return_dbus_error("org.freedesktop.NetworkManager.SecretAgent.UserCanceled", tr("The person canceled")),
        }
    });
}

fn register(c: &gio::DBusConnection) {
    let info = gio::DBusNodeInfo::for_xml(XML).expect("SecretAgent XML");
    let iface = info.lookup_interface("org.freedesktop.NetworkManager.SecretAgent").expect("SecretAgent interface");
    let r = c
        .register_object("/org/freedesktop/NetworkManager/SecretAgent", &iface)
        .method_call(move |_, _, _, _, method, params, inv| match method {
            "GetSecrets" => get_secrets(&params, inv),
            "CancelGetSecrets" => {
                // NetworkManager stopped waiting (the connection went away): the dialog closes
                let path: String = params.child_value(0).get::<glib::variant::ObjectPath>().map(|p| p.as_str().to_string()).unwrap_or_default();
                if debug() {
                    eprintln!("debug wifi: o NetworkManager desistiu ({path})");
                }
                if let Some((_, ssid)) = PENDING.with(|p| p.borrow().clone()).filter(|(p, _)| *p == path) {
                    wifi_password::withdraw(&ssid);
                }
                inv.return_value(None);
            }
            // NetworkManager keeps the secrets itself (system-owned); nothing to save or delete here
            "SaveSecrets" | "DeleteSecrets" => inv.return_value(None),
            _ => inv.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method),
        })
        .build();
    if let Err(e) = r {
        eprintln!("kalango-shell: agente de senhas: {e}");
    }
}

/// Becomes the session's secret agent, again each time NetworkManager (re)starts.
pub fn start() {
    let Some(bus) = bus_type() else { return };
    let Ok(c) = gio::bus_get_sync(bus, gio::Cancellable::NONE) else {
        eprintln!("kalango-shell: agente de senhas: sem o barramento");
        return;
    };
    register(&c);
    let watch = gio::bus_watch_name_on_connection(
        &c,
        "org.freedesktop.NetworkManager",
        gio::BusNameWatcherFlags::NONE,
        |c, _, _| {
            c.call(
                Some("org.freedesktop.NetworkManager"),
                "/org/freedesktop/NetworkManager/AgentManager",
                "org.freedesktop.NetworkManager.AgentManager",
                "RegisterWithCapabilities",
                Some(&("os.kalango.shell", 0u32).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                -1,
                gio::Cancellable::NONE,
                |r| match r {
                    Ok(_) => {
                        if debug() {
                            eprintln!("debug wifi: agente de senhas registrado no NetworkManager");
                        }
                    }
                    Err(e) => eprintln!("kalango-shell: agente de senhas: {e}"),
                },
            );
        },
        |_, _| {},
    );
    // (for the shell's whole life)
    std::mem::forget(watch);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_the_network_as_the_person_knows_it() {
        let mut c: Settings = HashMap::new();
        c.insert("802-11-wireless".into(), HashMap::from([("ssid".to_string(), b"Biblioteca".to_vec().to_variant())]));
        c.insert("802-11-wireless-security".into(), HashMap::from([("key-mgmt".to_string(), "sae".to_variant())]));
        assert_eq!(describe(&c), ("Biblioteca".into(), "WPA3".into(), false));
        c.insert("802-11-wireless-security".into(), HashMap::from([("key-mgmt".to_string(), "wpa-eap".to_variant())]));
        assert!(describe(&c).2);
    }

    #[test]
    fn replies_in_networkmanagers_shape() {
        let v = reply("802-11-wireless-security", "WPA2", None, "senha-certa".into());
        let m: HashMap<String, HashMap<String, glib::Variant>> = v.child_value(0).get().unwrap();
        assert_eq!(m["802-11-wireless-security"]["psk"].get::<String>().as_deref(), Some("senha-certa"));
        let v = reply("802-1x", "WPA2 Enterprise", Some("ana".into()), "x".into());
        let m: HashMap<String, HashMap<String, glib::Variant>> = v.child_value(0).get().unwrap();
        assert_eq!(m["802-1x"]["identity"].get::<String>().as_deref(), Some("ana"));
    }
}
