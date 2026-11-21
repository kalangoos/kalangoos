//! The window's accessible elements (AT-SPI, the accessibility bus): what can be clicked or typed into, with its
//! name, role and box — the agent clicks "e12" instead of guessing pixels, and kalangod sees a password field.
//! GTK 4 apps answer the whole tree in one call (org.a11y.atspi.Cache.GetItems); others are walked.

use gio::glib::Variant;
use gio::prelude::*;
use serde_json::{json, Value};

const ROOT: &str = "/org/a11y/atspi/accessible/root";
const TIMEOUT_MS: i32 = 1500;

// AT-SPI states (atspi-constants.h)
const ACTIVE: u32 = 1;
const EDITABLE: u32 = 7;
const FOCUSED: u32 = 12;
const SENSITIVE: u32 = 24;
const SHOWING: u32 = 25;
const VISIBLE: u32 = 30;
const CHECKED: u32 = 4;

/// The roles worth listing (AT-SPI role number → a short word for the model), and whether they are clicked.
pub fn role_word(role: u32) -> Option<(&'static str, bool)> {
    Some(match role {
        43 | 129 => ("button", true),
        62 => ("toggle", true),
        7 | 8 => ("checkbox", true),
        44 | 45 => ("radio", true),
        35 => ("menu item", true),
        33 => ("menu", true),
        37 => ("tab", true),
        11 => ("combo box", true),
        32 | 91 => ("list item", true),
        56 => ("cell", true),
        88 => ("link", true),
        51 => ("slider", true),
        52 => ("spin box", true),
        79 | 61 => ("text field", true),
        40 => ("password field", true),
        60 => ("terminal", true),
        26 => ("icon", true),
        29 => ("label", false),
        83 => ("heading", false),
        _ => return None,
    })
}

fn has(states: &[u32], bit: u32) -> bool {
    states.get((bit / 32) as usize).is_some_and(|w| w & (1 << (bit % 32)) != 0)
}

struct Bus {
    c: gio::DBusConnection,
}

impl Bus {
    fn open() -> Result<Bus, String> {
        let session = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).map_err(|e| e.to_string())?;
        let r = session.call_sync(Some("org.a11y.Bus"), "/org/a11y/bus", "org.a11y.Bus", "GetAddress", None, None, gio::DBusCallFlags::NONE, TIMEOUT_MS, gio::Cancellable::NONE)
            .map_err(|e| format!("accessibility bus: {e}"))?;
        let addr = r.child_value(0).str().unwrap_or("").to_string();
        let c = gio::DBusConnection::for_address_sync(&addr, gio::DBusConnectionFlags::AUTHENTICATION_CLIENT | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION, None, gio::Cancellable::NONE)
            .map_err(|e| format!("accessibility bus: {e}"))?;
        Ok(Bus { c })
    }

    fn call(&self, bus: &str, path: &str, iface: &str, method: &str, args: Option<&Variant>) -> Result<Variant, String> {
        self.c.call_sync(Some(bus), path, iface, method, args, None, gio::DBusCallFlags::NONE, TIMEOUT_MS, gio::Cancellable::NONE).map_err(|e| e.to_string())
    }

    fn children(&self, bus: &str, path: &str) -> Vec<(String, String)> {
        self.call(bus, path, "org.a11y.atspi.Accessible", "GetChildren", None).ok()
            .map(|v| v.child_value(0).iter().map(|c| (c.child_value(0).str().unwrap_or("").to_string(), c.child_value(1).str().unwrap_or("").to_string())).collect())
            .unwrap_or_default()
    }

    fn pid_of(&self, bus: &str) -> Option<i32> {
        let v = self.call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "GetConnectionUnixProcessID", Some(&(bus,).to_variant())).ok()?;
        v.child_value(0).get::<u32>().map(|p| p as i32)
    }

    fn name(&self, bus: &str, path: &str) -> String {
        self.call(bus, path, "org.freedesktop.DBus.Properties", "Get", Some(&("org.a11y.atspi.Accessible", "Name").to_variant())).ok()
            .and_then(|v| v.child_value(0).as_variant()).and_then(|v| v.str().map(str::to_string)).unwrap_or_default()
    }

    fn description(&self, bus: &str, path: &str) -> String {
        self.call(bus, path, "org.freedesktop.DBus.Properties", "Get", Some(&("org.a11y.atspi.Accessible", "Description").to_variant())).ok()
            .and_then(|v| v.child_value(0).as_variant()).and_then(|v| v.str().map(str::to_string)).unwrap_or_default()
    }

    fn role(&self, bus: &str, path: &str) -> u32 {
        self.call(bus, path, "org.a11y.atspi.Accessible", "GetRole", None).ok().and_then(|v| v.child_value(0).get::<u32>()).unwrap_or(0)
    }

    fn states(&self, bus: &str, path: &str) -> Vec<u32> {
        self.call(bus, path, "org.a11y.atspi.Accessible", "GetState", None).ok().and_then(|v| v.child_value(0).get::<Vec<u32>>()).unwrap_or_default()
    }

    /// (x, y, w, h) relative to the window.
    fn extents(&self, bus: &str, path: &str) -> Option<(i32, i32, i32, i32)> {
        let v = self.call(bus, path, "org.a11y.atspi.Component", "GetExtents", Some(&(1u32,).to_variant())).ok()?;
        v.child_value(0).get::<(i32, i32, i32, i32)>()
    }

    fn text(&self, bus: &str, path: &str) -> Option<String> {
        let v = self.call(bus, path, "org.a11y.atspi.Text", "GetText", Some(&(0i32, -1i32).to_variant())).ok()?;
        v.child_value(0).str().map(str::to_string)
    }
}

/// A node of the app's tree: (path, parent path, name, role, states, has the Text interface).
struct Node {
    path: String,
    parent: String,
    name: String,
    role: u32,
    states: Vec<u32>,
    text: bool,
}

/// GTK 4's cache: every node of the app in one call.
fn cached(b: &Bus, bus: &str) -> Option<Vec<Node>> {
    let v = b.call(bus, "/org/a11y/atspi/cache", "org.a11y.atspi.Cache", "GetItems", None).ok()?;
    let items = v.child_value(0);
    let mut out = Vec::new();
    for it in items.iter() {
        if it.n_children() < 10 {
            return None;
        }
        let path = it.child_value(0).child_value(1).str().unwrap_or("").to_string();
        let parent = it.child_value(2).child_value(1).str().unwrap_or("").to_string();
        let ifaces: Vec<String> = it.child_value(5).get().unwrap_or_default();
        out.push(Node {
            path,
            parent,
            name: it.child_value(6).str().unwrap_or("").to_string(),
            role: it.child_value(7).get::<u32>().unwrap_or(0),
            states: it.child_value(9).get::<Vec<u32>>().unwrap_or_default(),
            text: ifaces.iter().any(|i| i == "org.a11y.atspi.Text"),
        });
    }
    Some(out)
}

/// The tree walked one call at a time (apps without the cache): showing nodes only, at most `limit`.
fn walked(b: &Bus, bus: &str, window: &str, limit: usize) -> Vec<Node> {
    let mut out = Vec::new();
    let mut stack = vec![(window.to_string(), String::new(), 0usize)];
    while let Some((path, parent, depth)) = stack.pop() {
        if out.len() >= limit || depth > 40 {
            break;
        }
        let states = b.states(bus, &path);
        if !has(&states, SHOWING) && path != window {
            continue;
        }
        let role = b.role(bus, &path);
        let mut name = if role_word(role).is_some() || path == window { b.name(bus, &path) } else { String::new() };
        // (an icon button has no name: its tooltip, the description, says what it does)
        if name.is_empty() && role_word(role).is_some_and(|r| r.1) {
            name = b.description(bus, &path);
        }
        for (_, c) in b.children(bus, &path).into_iter().rev() {
            stack.push((c, path.clone(), depth + 1));
        }
        out.push(Node { path, parent, name, role, states, text: matches!(role, 61 | 79 | 40 | 60) });
    }
    out
}

/// The app with this process on the accessibility bus.
fn app_of(b: &Bus, pid: i32) -> Option<String> {
    b.children("org.a11y.atspi.Registry", ROOT).into_iter().map(|(bus, _)| bus).find(|bus| b.pid_of(bus) == Some(pid))
}

/// The elements of the window of process `pid` titled `title` (size `w × h`, to tell windows of one process apart).
/// Each: {role, name, x, y, w, h (window-relative), clickable, focused, editable, password, checked, value?}.
pub fn elements(pid: i32, title: &str, w: i32, h: i32, limit: usize) -> Result<Value, String> {
    let b = Bus::open()?;
    let bus = app_of(&b, pid).ok_or("this app is not on the accessibility bus")?;
    let windows = b.children(&bus, ROOT);
    // the window: its title, else the active one, else the one of this size
    let pick = windows.iter().map(|(_, p)| p.clone()).find(|p| !title.is_empty() && b.name(&bus, p) == title)
        .or_else(|| windows.iter().map(|(_, p)| p.clone()).find(|p| has(&b.states(&bus, p), ACTIVE)))
        .or_else(|| windows.iter().map(|(_, p)| p.clone()).find(|p| b.extents(&bus, p).is_some_and(|e| (e.2 - w).abs() < 4 && (e.3 - h).abs() < 4)))
        .ok_or("the window is not on the accessibility bus")?;
    // (GTK 4's cache only holds the objects already asked for — it registers them as they are walked — so the window
    // is walked; the cache is the fallback when walking finds nothing)
    let mut all = walked(&b, &bus, &pick, 1500);
    if all.len() <= 1 {
        all = cached(&b, &bus).unwrap_or_default();
    }
    // the window's own nodes, showing all the way up to it
    let by_path: std::collections::HashMap<&str, &Node> = all.iter().map(|n| (n.path.as_str(), n)).collect();
    let inside = |n: &Node| -> bool {
        let mut cur = n;
        for _ in 0..64 {
            if cur.path == pick {
                return true;
            }
            if !(has(&cur.states, SHOWING) && has(&cur.states, VISIBLE)) {
                return false;
            }
            match by_path.get(cur.parent.as_str()) {
                Some(p) => cur = p,
                None => return cur.parent == pick,
            }
        }
        false
    };
    let mut out = Vec::new();
    let mut texts = 0;
    for n in all.iter().filter(|n| n.path != pick) {
        let Some((word, clickable)) = role_word(n.role) else { continue };
        if !inside(n) {
            continue;
        }
        let editable = has(&n.states, EDITABLE);
        let password = n.role == 40;
        if !clickable && n.name.trim().is_empty() {
            continue;
        }
        let Some((x, y, ew, eh)) = b.extents(&bus, &n.path) else { continue };
        // (inside the window and of some size: what was scrolled away is not there to click)
        if ew < 2 || eh < 2 || x + ew <= 0 || y + eh <= 0 || x >= w || y >= h {
            continue;
        }
        let mut e = json!({
            "papel": word, "nome": n.name.chars().take(120).collect::<String>(), "x": x, "y": y, "w": ew, "h": eh,
            "clicavel": clickable && has(&n.states, SENSITIVE), "foco": has(&n.states, FOCUSED), "editavel": editable, "senha": password,
        });
        if matches!(n.role, 7 | 8 | 44 | 45 | 62) {
            e["marcado"] = json!(has(&n.states, CHECKED));
        }
        // what an editable field holds (never a password's)
        if editable && !password && n.text && texts < 4 {
            if let Some(t) = b.text(&bus, &n.path) {
                texts += 1;
                e["valor"] = json!(t.chars().take(400).collect::<String>());
            }
        }
        out.push(e);
        if out.len() >= limit {
            break;
        }
    }
    Ok(json!({"elementos": out, "nos": all.len()}))
}
