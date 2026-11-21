//! The Wayland side: niri offers wlr-screencopy, the wlr virtual pointer and the zwp virtual keyboard only to the
//! processes kalangod wrote in its allow file (patch 0008) — this helper, while a session lasts. Nothing here reads
//! the person's own input: the protocols only send.

use crate::keys;
use std::io::Write;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::os::unix::fs::FileExt;
use std::time::{Duration, Instant};
use wayland_client::protocol::{wl_buffer, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{zwp_virtual_keyboard_manager_v1 as vkm, zwp_virtual_keyboard_v1 as vk};
use wayland_protocols_wlr::screencopy::v1::client::{zwlr_screencopy_frame_v1 as frame, zwlr_screencopy_manager_v1 as scm};
use wayland_protocols_wlr::virtual_pointer::v1::client::{zwlr_virtual_pointer_manager_v1 as vpm, zwlr_virtual_pointer_v1 as vp};

#[derive(Default)]
pub struct Output {
    pub wl: Option<wl_output::WlOutput>,
    pub name: String,
    /// the output is flipped upside down (niri's nested window: its frames come with the rows reversed)
    pub flipped: bool,
    pub transform: Transform,
}

/// An output's transform (wl_output), as the compositor applies it to absolute pointer positions.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Transform {
    #[default]
    Normal,
    R90,
    R180,
    R270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

impl Transform {
    fn from_wl(t: wl_output::Transform) -> Transform {
        match t {
            wl_output::Transform::_90 => Transform::R90,
            wl_output::Transform::_180 => Transform::R180,
            wl_output::Transform::_270 => Transform::R270,
            wl_output::Transform::Flipped => Transform::Flipped,
            wl_output::Transform::Flipped90 => Transform::Flipped90,
            wl_output::Transform::Flipped180 => Transform::Flipped180,
            wl_output::Transform::Flipped270 => Transform::Flipped270,
            _ => Transform::Normal,
        }
    }

    fn swaps(self) -> bool {
        matches!(self, Transform::R90 | Transform::R270 | Transform::Flipped90 | Transform::Flipped270)
    }

    /// The compositor's own mapping (smithay's `transform_point_in`): a point of the panel's untransformed area
    /// `(aw, ah)` → the output's logical point (the tests check `unapply` against it).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn apply(self, x: f64, y: f64, aw: f64, ah: f64) -> (f64, f64) {
        match self {
            Transform::Normal => (x, y),
            Transform::R90 => (ah - y, x),
            Transform::R180 => (aw - x, ah - y),
            Transform::R270 => (y, aw - x),
            Transform::Flipped => (aw - x, y),
            Transform::Flipped90 => (y, x),
            Transform::Flipped180 => (x, ah - y),
            Transform::Flipped270 => (ah - y, aw - x),
        }
    }

    /// What to send for the logical point (x, y) of an output of logical size `w × h`: the point of the panel's
    /// area and that area's size (the compositor transforms it back to (x, y)).
    pub fn unapply(self, x: f64, y: f64, w: f64, h: f64) -> (f64, f64, f64, f64) {
        let (aw, ah) = if self.swaps() { (h, w) } else { (w, h) };
        let (px, py) = match self {
            Transform::Normal => (x, y),
            Transform::R90 => (y, ah - x),
            Transform::R180 => (aw - x, ah - y),
            Transform::R270 => (aw - y, x),
            Transform::Flipped => (aw - x, y),
            Transform::Flipped90 => (y, x),
            Transform::Flipped180 => (x, ah - y),
            Transform::Flipped270 => (aw - y, ah - x),
        };
        (px, py, aw, ah)
    }
}

#[derive(Default)]
struct Frame {
    format: Option<wl_shm::Format>,
    w: u32,
    h: u32,
    stride: u32,
    buffer_done: bool,
    ready: bool,
    failed: bool,
    y_invert: bool,
}

#[derive(Default)]
pub struct State {
    seat: Option<wl_seat::WlSeat>,
    shm: Option<wl_shm::WlShm>,
    pub outputs: Vec<Output>,
    screencopy: Option<scm::ZwlrScreencopyManagerV1>,
    pointer_manager: Option<vpm::ZwlrVirtualPointerManagerV1>,
    keyboard_manager: Option<vkm::ZwpVirtualKeyboardManagerV1>,
    frame: Frame,
}

/// A captured output: its pixels (BGRx or RGBx), size and stride.
pub struct Capture {
    pub data: Vec<u8>,
    pub w: u32,
    pub h: u32,
    pub stride: u32,
    pub bgrx: bool,
    pub y_invert: bool,
}

pub struct Wl {
    con: Connection,
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    pub state: State,
    pointer: Option<(vp::ZwlrVirtualPointerV1, String)>,
    keyboard: Option<vk::ZwpVirtualKeyboardV1>,
    start: Instant,
}

impl Wl {
    pub fn connect() -> Result<Wl, String> {
        let con = Connection::connect_to_env().map_err(|e| format!("wayland: {e}"))?;
        let mut queue = con.new_event_queue();
        let qh = queue.handle();
        let _registry = con.display().get_registry(&qh, ());
        let mut state = State::default();
        queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
        // (the outputs' names come after binding them)
        queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
        Ok(Wl { con, queue, qh, state, pointer: None, keyboard: None, start: Instant::now() })
    }

    /// What the compositor offered this process (the gate decides).
    pub fn capabilities(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.state.screencopy.is_some() {
            v.push("captura");
        }
        if self.state.pointer_manager.is_some() {
            v.push("ponteiro");
        }
        if self.state.keyboard_manager.is_some() {
            v.push("teclado");
        }
        v
    }

    /// The compositor's process (the peer of the Wayland socket), to check it is the same niri as the IPC's.
    pub fn compositor_pid(&self) -> Option<i32> {
        use std::os::fd::AsRawFd;
        let fd = self.con.backend().poll_fd().as_raw_fd();
        crate::peer_pid(fd)
    }

    fn time(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    fn output(&self, name: &str) -> Result<&wl_output::WlOutput, String> {
        self.state.outputs.iter().find(|o| o.name == name || name.is_empty()).and_then(|o| o.wl.as_ref()).ok_or_else(|| format!("no output {name:?}"))
    }

    pub fn capture(&mut self, output: &str) -> Result<Capture, String> {
        let m = self.state.screencopy.clone().ok_or("screen capture not allowed")?;
        let shm = self.state.shm.clone().ok_or("no wl_shm")?;
        let out = self.output(output)?.clone();
        let flipped = self.state.outputs.iter().any(|o| o.wl.as_ref() == Some(&out) && o.flipped);
        self.state.frame = Frame::default();
        let fr = m.capture_output(0, &out, &self.qh, ());
        let t0 = Instant::now();
        while !self.state.frame.buffer_done && !self.state.frame.failed {
            self.queue.blocking_dispatch(&mut self.state).map_err(|e| e.to_string())?;
            if t0.elapsed() > Duration::from_secs(5) {
                return Err("the compositor did not describe the frame".into());
            }
        }
        let f = &self.state.frame;
        let format = f.format.ok_or("no shm format offered")?;
        let (w, h, stride) = (f.w, f.h, f.stride);
        let size = (stride * h) as usize;
        let fd = unsafe { libc::memfd_create(c"kalango-tela".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return Err("memfd_create failed".into());
        }
        let file = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) });
        file.set_len(size as u64).map_err(|e| e.to_string())?;
        let pool = shm.create_pool(file.as_fd(), size as i32, &self.qh, ());
        let buf = pool.create_buffer(0, w as i32, h as i32, stride as i32, format, &self.qh, ());
        fr.copy(&buf);
        while !self.state.frame.ready && !self.state.frame.failed {
            self.queue.blocking_dispatch(&mut self.state).map_err(|e| e.to_string())?;
            if t0.elapsed() > Duration::from_secs(5) {
                return Err("the compositor did not copy the frame".into());
            }
        }
        let failed = self.state.frame.failed;
        let y_invert = self.state.frame.y_invert != flipped;
        fr.destroy();
        buf.destroy();
        pool.destroy();
        if failed {
            return Err("the compositor refused the capture".into());
        }
        let mut data = vec![0u8; size];
        file.read_exact_at(&mut data, 0).map_err(|e| e.to_string())?;
        let bgrx = matches!(format, wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888);
        Ok(Capture { data, w, h, stride, bgrx, y_invert })
    }

    fn pointer(&mut self, output: &str) -> Result<vp::ZwlrVirtualPointerV1, String> {
        if let Some((p, o)) = &self.pointer {
            if o == output {
                return Ok(p.clone());
            }
        }
        let m = self.state.pointer_manager.clone().ok_or("the pointer is not allowed")?;
        let out = self.output(output)?.clone();
        let p = if m.version() >= 2 {
            m.create_virtual_pointer_with_output(self.state.seat.as_ref(), Some(&out), &self.qh, ())
        } else {
            m.create_virtual_pointer(self.state.seat.as_ref(), &self.qh, ())
        };
        self.pointer = Some((p.clone(), output.to_string()));
        Ok(p)
    }

    /// Moves the pointer to (x, y) of the output, in its logical size `w × h`.
    pub fn move_to(&mut self, output: &str, x: f64, y: f64, w: u32, h: u32) -> Result<(), String> {
        let p = self.pointer(output)?;
        let t = self.time();
        // (the compositor turns an absolute position by the output's transform: the inverse goes out)
        let tr = self.state.outputs.iter().find(|o| o.name == output || output.is_empty()).map(|o| o.transform).unwrap_or_default();
        let (x, y) = (x.clamp(0.0, (w - 1) as f64), y.clamp(0.0, (h - 1) as f64));
        let (px, py, aw, ah) = tr.unapply(x, y, w as f64, h as f64);
        p.motion_absolute(t, px.round() as u32, py.round() as u32, aw as u32, ah as u32);
        p.frame();
        self.flush()
    }

    /// BTN_LEFT 0x110, BTN_RIGHT 0x111, BTN_MIDDLE 0x112.
    pub fn button(&mut self, output: &str, code: u32, pressed: bool) -> Result<(), String> {
        let p = self.pointer(output)?;
        let t = self.time();
        p.button(t, code, if pressed { wl_pointer::ButtonState::Pressed } else { wl_pointer::ButtonState::Released });
        p.frame();
        self.flush()
    }

    /// Wheel steps: `dy` > 0 scrolls down, `dx` > 0 right.
    pub fn scroll(&mut self, output: &str, dx: i32, dy: i32) -> Result<(), String> {
        let p = self.pointer(output)?;
        for (axis, n) in [(wl_pointer::Axis::VerticalScroll, dy), (wl_pointer::Axis::HorizontalScroll, dx)] {
            for _ in 0..n.unsigned_abs() {
                let t = self.time();
                p.axis_source(wl_pointer::AxisSource::Wheel);
                p.axis_discrete(t, axis, 15.0 * n.signum() as f64, n.signum());
                p.frame();
                self.flush()?;
                std::thread::sleep(Duration::from_millis(16));
            }
        }
        Ok(())
    }

    fn keyboard(&mut self) -> Result<vk::ZwpVirtualKeyboardV1, String> {
        if let Some(k) = &self.keyboard {
            return Ok(k.clone());
        }
        let m = self.state.keyboard_manager.clone().ok_or("the keyboard is not allowed")?;
        let seat = self.state.seat.clone().ok_or("no seat")?;
        let k = m.create_virtual_keyboard(&seat, &self.qh, ());
        self.keyboard = Some(k.clone());
        Ok(k)
    }

    fn set_keymap(&mut self, k: &vk::ZwpVirtualKeyboardV1, symbols: &[String]) -> Result<(), String> {
        let text = keys::keymap(symbols);
        let fd = unsafe { libc::memfd_create(c"kalango-tela-xkb".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return Err("memfd_create failed".into());
        }
        let mut f = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) });
        f.write_all(text.as_bytes()).and_then(|_| f.write_all(&[0])).map_err(|e| e.to_string())?;
        k.keymap(1, f.as_fd(), (text.len() + 1) as u32);
        self.flush()
    }

    fn tap(&mut self, k: &vk::ZwpVirtualKeyboardV1, code: u32) -> Result<(), String> {
        let t = self.time();
        k.key(t, code, 1);
        k.key(t + 1, code, 0);
        self.flush()?;
        // (a breath between keys: apps see each one, and nothing repeats)
        std::thread::sleep(Duration::from_millis(6));
        Ok(())
    }

    pub fn type_text(&mut self, text: &str) -> Result<(), String> {
        let k = self.keyboard()?;
        for (symbols, seq) in keys::chunks(text) {
            self.set_keymap(&k, &symbols)?;
            k.modifiers(0, 0, 0, 0);
            for i in seq {
                self.tap(&k, keys::evdev_of_symbol(i))?;
            }
        }
        self.roundtrip()
    }

    pub fn combo(&mut self, c: &keys::Combo) -> Result<(), String> {
        let k = self.keyboard()?;
        self.set_keymap(&k, std::slice::from_ref(&c.keysym))?;
        let mut mask = 0;
        let t = self.time();
        for m in &c.modifiers {
            let (code, bit) = keys::modifier_evdev(m);
            k.key(t, code, 1);
            mask |= bit;
        }
        k.modifiers(mask, 0, 0, 0);
        self.flush()?;
        self.tap(&k, keys::evdev_of_symbol(0))?;
        let t = self.time();
        for m in c.modifiers.iter().rev() {
            k.key(t, keys::modifier_evdev(m).0, 0);
        }
        k.modifiers(0, 0, 0, 0);
        self.roundtrip()
    }

    pub fn flush(&self) -> Result<(), String> {
        self.con.flush().map_err(|e| e.to_string())
    }

    pub fn roundtrip(&mut self) -> Result<(), String> {
        self.queue.roundtrip(&mut self.state).map(|_| ()).map_err(|e| e.to_string())
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(s: &mut Self, r: &wl_registry::WlRegistry, ev: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = ev {
            match interface.as_str() {
                "wl_seat" if s.seat.is_none() => s.seat = Some(r.bind(name, version.min(7), qh, ())),
                "wl_shm" => s.shm = Some(r.bind(name, 1, qh, ())),
                "wl_output" => {
                    let i = s.outputs.len();
                    s.outputs.push(Output { wl: Some(r.bind(name, version.min(4), qh, i)), name: String::new(), flipped: false, transform: Transform::Normal });
                }
                "zwlr_screencopy_manager_v1" => s.screencopy = Some(r.bind(name, version.min(3), qh, ())),
                "zwlr_virtual_pointer_manager_v1" => s.pointer_manager = Some(r.bind(name, version.min(2), qh, ())),
                "zwp_virtual_keyboard_manager_v1" => s.keyboard_manager = Some(r.bind(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, usize> for State {
    fn event(s: &mut Self, _: &wl_output::WlOutput, ev: wl_output::Event, i: &usize, _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            wl_output::Event::Name { name } => {
                if let Some(o) = s.outputs.get_mut(*i) {
                    o.name = name;
                }
            }
            wl_output::Event::Geometry { transform, .. } => {
                if let Some(o) = s.outputs.get_mut(*i) {
                    o.flipped = matches!(transform, WEnum::Value(wl_output::Transform::Flipped180));
                    if let WEnum::Value(t) = transform {
                        o.transform = Transform::from_wl(t);
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<frame::ZwlrScreencopyFrameV1, ()> for State {
    fn event(s: &mut Self, _: &frame::ZwlrScreencopyFrameV1, ev: frame::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            frame::Event::Buffer { format: WEnum::Value(f), width, height, stride } => {
                // (the first 8-bit RGB format offered)
                if s.frame.format.is_none() && matches!(f, wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888 | wl_shm::Format::Xbgr8888 | wl_shm::Format::Abgr8888) {
                    s.frame.format = Some(f);
                    s.frame.w = width;
                    s.frame.h = height;
                    s.frame.stride = stride;
                }
            }
            frame::Event::BufferDone => s.frame.buffer_done = true,
            frame::Event::Flags { flags: WEnum::Value(f) } => s.frame.y_invert = f.contains(frame::Flags::YInvert),
            frame::Event::Ready { .. } => s.frame.ready = true,
            frame::Event::Failed => s.frame.failed = true,
            _ => {}
        }
        // (a version 1 frame sends no buffer_done: the buffer event is the last description)
        if s.frame.format.is_some() && s.screencopy.as_ref().is_some_and(|m| m.version() < 3) {
            s.frame.buffer_done = true;
        }
    }
}

macro_rules! ignore {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for State {
            fn event(_: &mut Self, _: &$t, _: <$t as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
ignore!(wl_seat::WlSeat, wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_buffer::WlBuffer, scm::ZwlrScreencopyManagerV1, vpm::ZwlrVirtualPointerManagerV1,
    vp::ZwlrVirtualPointerV1, vkm::ZwpVirtualKeyboardManagerV1, vk::ZwpVirtualKeyboardV1);

#[cfg(test)]
mod tests {
    use super::Transform;

    #[test]
    fn absolute_positions_survive_the_output_transform() {
        let all = [Transform::Normal, Transform::R90, Transform::R180, Transform::R270, Transform::Flipped, Transform::Flipped90, Transform::Flipped180, Transform::Flipped270];
        for t in all {
            let (px, py, aw, ah) = t.unapply(300.0, 100.0, 1920.0, 1080.0);
            assert_eq!(t.apply(px, py, aw, ah), (300.0, 100.0), "{t:?}");
        }
        // niri's nested window is Flipped180: a point near the top goes out near the bottom
        assert_eq!(Transform::Flipped180.unapply(711.0, 492.0, 1920.0, 1080.0), (711.0, 588.0, 1920.0, 1080.0));
    }
}
