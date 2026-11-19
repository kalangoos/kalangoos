//! The shell as BlueZ's pairing agent (org.bluez.Agent1, the default one, "KeyboardDisplay"), as GNOME Shell is:
//! a keyboard shows the code to type on it, a phone the code to compare, an old device asks for its PIN, a device that
//! asks to pair is allowed or refused — all in the pairing dialog (bt_pairing.rs). Settings › Bluetooth pairs through
//! BlueZ on the same connection (`pair`), so its pairings come here too.
//!
//! Not in the development session (the system's BlueZ belongs to that desktop's own agent); the session test uses a
//! stand-in on the session bus (KALANGO_BLUEZ_BUS=session).

use crate::bt_pairing::{self, Mode};
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;

const PATH: &str = "/os/kalango/BluezAgent";

const XML: &str = r#"<node>
  <interface name="org.bluez.Agent1">
    <method name="Release"/>
    <method name="RequestPinCode"><arg name="device" type="o" direction="in"/><arg name="pincode" type="s" direction="out"/></method>
    <method name="DisplayPinCode"><arg name="device" type="o" direction="in"/><arg name="pincode" type="s" direction="in"/></method>
    <method name="RequestPasskey"><arg name="device" type="o" direction="in"/><arg name="passkey" type="u" direction="out"/></method>
    <method name="DisplayPasskey"><arg name="device" type="o" direction="in"/><arg name="passkey" type="u" direction="in"/><arg name="entered" type="q" direction="in"/></method>
    <method name="RequestConfirmation"><arg name="device" type="o" direction="in"/><arg name="passkey" type="u" direction="in"/></method>
    <method name="RequestAuthorization"><arg name="device" type="o" direction="in"/></method>
    <method name="AuthorizeService"><arg name="device" type="o" direction="in"/><arg name="uuid" type="s" direction="in"/></method>
    <method name="Cancel"/>
  </interface>
</node>"#;

thread_local! {
    static CONN: RefCell<Option<gio::DBusConnection>> = const { RefCell::new(None) };
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

fn bus_type() -> Option<gio::BusType> {
    match std::env::var("KALANGO_BLUEZ_BUS").as_deref() {
        Ok("session") => Some(gio::BusType::Session),
        _ if std::env::var_os("KALANGO_SHELL_DEV").is_some() => None,
        _ => Some(gio::BusType::System),
    }
}

/// The agent's connection, when the shell is BlueZ's agent here (pairing then goes through it).
pub fn connection() -> Option<gio::DBusConnection> {
    CONN.with(|c| c.borrow().clone())
}

/// A device's name and icon, from BlueZ.
fn device_info(c: &gio::DBusConnection, device: &str) -> (String, String, bool) {
    let r = c.call_sync(Some("org.bluez"), device, "org.freedesktop.DBus.Properties", "GetAll", Some(&("org.bluez.Device1",).to_variant()),
        Some(glib::VariantTy::new("(a{sv})").unwrap()), gio::DBusCallFlags::NONE, 2000, gio::Cancellable::NONE);
    let props: HashMap<String, glib::Variant> = r.ok().and_then(|v| v.child_value(0).get()).unwrap_or_default();
    let s = |k: &str| props.get(k).and_then(|v| v.get::<String>());
    let name = s("Alias").or_else(|| s("Name")).unwrap_or_else(|| device.rsplit('/').next().unwrap_or("").trim_start_matches("dev_").replace('_', ":"));
    let paired = props.get("Paired").and_then(|v| v.get::<bool>()).unwrap_or(false);
    (name, s("Icon").unwrap_or_default(), paired)
}

/// "Cancelar" while the device types the code: BlueZ stops pairing.
fn cancel_pairing(device: &str) {
    CONN.with(|c| {
        if let Some(c) = c.borrow().as_ref() {
            c.call(Some("org.bluez"), device, "org.bluez.Device1", "CancelPairing", None, None, gio::DBusCallFlags::NONE, 5000, gio::Cancellable::NONE, |_| {});
        }
    });
}

const REJECTED: &str = "org.bluez.Error.Rejected";
const CANCELED: &str = "org.bluez.Error.Canceled";

fn method(c: &gio::DBusConnection, name: &str, params: &glib::Variant, inv: gio::DBusMethodInvocation) {
    let device = || params.child_value(0).get::<glib::variant::ObjectPath>().map(|p| p.as_str().to_string()).unwrap_or_default();
    let ask = |mode: Mode, inv: gio::DBusMethodInvocation, reply: fn(&str) -> Option<glib::Variant>| {
        let dev = device();
        let (name, icon, _) = device_info(c, &dev);
        bt_pairing::show(&dev, &name, &icon, mode, Some(Box::new(move |answer: Option<String>| match answer {
            Some(a) => match reply(&a) {
                Some(v) => inv.return_value(Some(&v)),
                None => inv.return_value(None),
            },
            None => inv.return_dbus_error(REJECTED, "the person said no"),
        })), cancel_pairing);
    };
    if debug() {
        eprintln!("debug bluetooth: o BlueZ chama {name}");
    }
    match name {
        "Release" => inv.return_value(None),
        "Cancel" => {
            bt_pairing::withdraw();
            inv.return_value(None);
        }
        "DisplayPasskey" => {
            let dev = device();
            let passkey: u32 = params.child_value(1).get().unwrap_or(0);
            let entered: u16 = params.child_value(2).get().unwrap_or(0);
            let (name, icon, _) = device_info(c, &dev);
            bt_pairing::show(&dev, &name, &icon, Mode::Type { code: format!("{passkey:06}"), entered: Some(entered as usize) }, None, cancel_pairing);
            inv.return_value(None);
        }
        "DisplayPinCode" => {
            let dev = device();
            let pin: String = params.child_value(1).get().unwrap_or_default();
            let (name, icon, _) = device_info(c, &dev);
            bt_pairing::show(&dev, &name, &icon, Mode::Type { code: pin, entered: None }, None, cancel_pairing);
            inv.return_value(None);
        }
        "RequestConfirmation" => {
            let passkey: u32 = params.child_value(1).get().unwrap_or(0);
            ask(Mode::Compare { code: format!("{passkey:06}") }, inv, |_| None);
        }
        "RequestAuthorization" => ask(Mode::Allow, inv, |_| None),
        "RequestPinCode" => ask(Mode::Enter, inv, |pin| Some((pin.to_string(),).to_variant())),
        "RequestPasskey" => ask(Mode::Enter, inv, |pin| Some((pin.parse::<u32>().unwrap_or(0),).to_variant())),
        "AuthorizeService" => {
            // a paired device using one of its services (audio, input): yes; one never paired asks the person
            let dev = device();
            let (_, _, paired) = device_info(c, &dev);
            if paired {
                inv.return_value(None);
            } else {
                ask(Mode::Allow, inv, |_| None);
            }
        }
        _ => inv.return_dbus_error(CANCELED, name),
    }
}

/// Pairs a device (its address) through BlueZ on the agent's connection (`connection()`), so the agent answers its
/// questions. Blocking: call it from a worker thread.
pub fn pair_on(c: &gio::DBusConnection, mac: &str) -> Result<(), String> {
    let path = device_path(c, mac).ok_or_else(|| format!("{mac}: not found"))?;
    c.call_sync(Some("org.bluez"), &path, "org.bluez.Device1", "Pair", None, None, gio::DBusCallFlags::NONE, 90_000, gio::Cancellable::NONE)
        .map(|_| ())
        .map_err(|e| e.message().to_string())
}

/// The device's object, found by its address among BlueZ's objects.
fn device_path(c: &gio::DBusConnection, mac: &str) -> Option<String> {
    let v = c.call_sync(Some("org.bluez"), "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", None,
        Some(glib::VariantTy::new("(a{oa{sa{sv}}})").unwrap()), gio::DBusCallFlags::NONE, 5000, gio::Cancellable::NONE).ok()?;
    let objs: HashMap<glib::variant::ObjectPath, HashMap<String, HashMap<String, glib::Variant>>> = v.child_value(0).get()?;
    objs.into_iter().find_map(|(p, ifaces)| {
        let addr = ifaces.get("org.bluez.Device1")?.get("Address")?.get::<String>()?;
        addr.eq_ignore_ascii_case(mac).then(|| p.as_str().to_string())
    })
}

/// Becomes BlueZ's default agent, again each time bluetoothd (re)starts; and follows "Paired" to close the dialog
/// with its ✓.
pub fn start() {
    let Some(bus) = bus_type() else { return };
    let Ok(c) = gio::bus_get_sync(bus, gio::Cancellable::NONE) else {
        eprintln!("kalango-shell: agente do Bluetooth: sem o barramento");
        return;
    };
    let info = gio::DBusNodeInfo::for_xml(XML).expect("Agent1 XML");
    let iface = info.lookup_interface("org.bluez.Agent1").expect("Agent1 interface");
    let c2 = c.clone();
    let r = c.register_object(PATH, &iface).method_call(move |_, _, _, _, name, params, inv| method(&c2, name, &params, inv)).build();
    if let Err(e) = r {
        eprintln!("kalango-shell: agente do Bluetooth: {e}");
        return;
    }
    CONN.with(|x| *x.borrow_mut() = Some(c.clone()));
    // a device became paired: the dialog's ✓
    c.signal_subscribe(Some("org.bluez"), Some("org.freedesktop.DBus.Properties"), Some("PropertiesChanged"), None, Some("org.bluez.Device1"),
        gio::DBusSignalFlags::NONE, |conn, _, path, _, _, params| {
            let changed: HashMap<String, glib::Variant> = params.child_value(1).get().unwrap_or_default();
            if changed.get("Paired").and_then(|v| v.get::<bool>()) == Some(true) {
                bt_pairing::paired(path);
            }
            // connected or gone: the indicator with its name, kind and battery
            if let Some(on) = changed.get("Connected").and_then(|v| v.get::<bool>()) {
                avisar_conexao(conn, path, on);
            }
        });
    let watch = gio::bus_watch_name_on_connection(&c, "org.bluez", gio::BusNameWatcherFlags::NONE, |c, _, _| {
        let path = glib::variant::ObjectPath::try_from(PATH).expect("agent path");
        let c2 = c.clone();
        c.call(Some("org.bluez"), "/org/bluez", "org.bluez.AgentManager1", "RegisterAgent", Some(&(path.clone(), "KeyboardDisplay").to_variant()),
            None, gio::DBusCallFlags::NONE, 5000, gio::Cancellable::NONE, move |r| {
                if let Err(e) = r {
                    eprintln!("kalango-shell: agente do Bluetooth: {e}");
                    return;
                }
                c2.call(Some("org.bluez"), "/org/bluez", "org.bluez.AgentManager1", "RequestDefaultAgent", Some(&(path,).to_variant()),
                    None, gio::DBusCallFlags::NONE, 5000, gio::Cancellable::NONE, |r| match r {
                        Ok(_) => {
                            if debug() {
                                eprintln!("debug bluetooth: agente de pareamento registrado no BlueZ");
                            }
                        }
                        Err(e) => eprintln!("kalango-shell: agente do Bluetooth: {e}"),
                    });
            });
    }, |_, _| {});
    std::mem::forget(watch);
}

/// A device's name, kind and battery (BlueZ's Device1 and Battery1), then the indicator.
fn avisar_conexao(c: &gio::DBusConnection, path: &str, on: bool) {
    let (c2, p) = (c.clone(), path.to_string());
    c.call(Some("org.bluez"), path, "org.freedesktop.DBus.Properties", "GetAll", Some(&("org.bluez.Device1",).to_variant()),
        Some(glib::VariantTy::new("(a{sv})").expect("type")), gio::DBusCallFlags::NONE, 3000, gio::Cancellable::NONE, move |r| {
            let props: HashMap<String, glib::Variant> = r.ok().and_then(|v| v.child_value(0).get()).unwrap_or_default();
            let texto = |k: &str| props.get(k).and_then(|v| v.get::<String>());
            let nome = texto("Alias").or_else(|| texto("Name")).unwrap_or_else(|| kalango_i18n::tr("A device").to_string());
            let icone = texto("Icon").unwrap_or_default();
            c2.call(Some("org.bluez"), &p, "org.freedesktop.DBus.Properties", "Get", Some(&("org.bluez.Battery1", "Percentage").to_variant()),
                Some(glib::VariantTy::new("(v)").expect("type")), gio::DBusCallFlags::NONE, 3000, gio::Cancellable::NONE, move |r| {
                    let bateria = r.ok().and_then(|v| v.child_value(0).as_variant()).and_then(|v| v.get::<u8>());
                    crate::osd::dispositivo(&nome, &icone, on, bateria);
                });
        });
}
