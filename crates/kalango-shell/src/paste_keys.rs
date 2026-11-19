//! Pastes into the focused window as the person would: Ctrl+V (Ctrl+Shift+V in a terminal) through the
//! compositor's virtual keyboard (zwp_virtual_keyboard_v1), once the clipboard holds what was chosen.

use std::io::Write;
use std::os::fd::AsFd;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{zwp_virtual_keyboard_manager_v1 as manager, zwp_virtual_keyboard_v1 as keyboard};

/// Just the keys a paste needs (the compositor fills in the rest from its own data).
const KEYMAP: &str = "xkb_keymap {
xkb_keycodes \"kalango\" { minimum = 8; maximum = 255; <LCTL> = 37; <LFSH> = 50; <AB04> = 55; };
xkb_types \"kalango\" { include \"complete\" };
xkb_compat \"kalango\" { include \"complete\" };
xkb_symbols \"kalango\" {
  key <LCTL> { [ Control_L ] };
  key <LFSH> { [ Shift_L ] };
  key <AB04> { [ v, V ] };
  modifier_map Control { <LCTL> };
  modifier_map Shift { <LFSH> };
};
};
";

// evdev codes (the keymap's less 8) and the modifier masks
const CTRL: u32 = 29;
const SHIFT: u32 = 42;
const V: u32 = 47;
const MASK_SHIFT: u32 = 1;
const MASK_CTRL: u32 = 4;

struct State {
    seat: Option<wl_seat::WlSeat>,
    manager: Option<manager::ZwpVirtualKeyboardManagerV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(s: &mut Self, r: &wl_registry::WlRegistry, ev: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = ev {
            match interface.as_str() {
                "wl_seat" if s.seat.is_none() => s.seat = Some(r.bind(name, version.min(7), qh, ())),
                "zwp_virtual_keyboard_manager_v1" => s.manager = Some(r.bind(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<manager::ZwpVirtualKeyboardManagerV1, ()> for State {
    fn event(_: &mut Self, _: &manager::ZwpVirtualKeyboardManagerV1, _: manager::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<keyboard::ZwpVirtualKeyboardV1, ()> for State {
    fn event(_: &mut Self, _: &keyboard::ZwpVirtualKeyboardV1, _: keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

/// Ctrl+V into the focused window (with Shift for a terminal). Blocking for a few milliseconds.
pub fn paste(shift: bool) -> Result<(), String> {
    let con = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let mut queue = con.new_event_queue();
    let qh = queue.handle();
    let _registry = con.display().get_registry(&qh, ());
    let mut s = State { seat: None, manager: None };
    queue.roundtrip(&mut s).map_err(|e| e.to_string())?;
    let (Some(seat), Some(m)) = (s.seat.clone(), s.manager.clone()) else {
        return Err("the compositor has no virtual keyboard".into());
    };
    let kb = m.create_virtual_keyboard(&seat, &qh, ());
    // the keymap goes as a file (with its closing NUL)
    let path = gtk::glib::user_runtime_dir().join(format!("kalango-colar-{}.xkb", std::process::id()));
    let mut f = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    f.write_all(KEYMAP.as_bytes()).and_then(|_| f.write_all(&[0])).map_err(|e| e.to_string())?;
    let f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&path);
    kb.keymap(1, f.as_fd(), (KEYMAP.len() + 1) as u32);
    let t = |n: u32| n;
    let mods = MASK_CTRL | if shift { MASK_SHIFT } else { 0 };
    kb.key(t(0), CTRL, 1);
    if shift {
        kb.key(t(1), SHIFT, 1);
    }
    kb.modifiers(mods, 0, 0, 0);
    kb.key(t(2), V, 1);
    kb.key(t(3), V, 0);
    if shift {
        kb.key(t(4), SHIFT, 0);
    }
    kb.key(t(5), CTRL, 0);
    kb.modifiers(0, 0, 0, 0);
    queue.roundtrip(&mut s).map_err(|e| e.to_string())?;
    kb.destroy();
    let _ = con.flush();
    Ok(())
}
