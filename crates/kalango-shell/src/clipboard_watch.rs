//! Watches the session's clipboard (the compositor's ext-data-control-v1, as a clipboard manager does): every new
//! selection — text, a picture, a colour — comes to the shell for the clipboard history (Super+V). What a password
//! manager marks as secret (`x-kde-passwordManagerHint`) never comes: passwords never enter the history.
//!
//! A Wayland connection of its own, in a thread; each selection's content is read from its pipe in another thread
//! (the app that copied writes it whenever it can).

use std::collections::HashMap;
use std::io::Read;
use std::os::fd::AsFd;
use std::sync::mpsc;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::data_control::v1::client::{ext_data_control_device_v1 as device, ext_data_control_manager_v1 as manager, ext_data_control_offer_v1 as offer};

#[derive(Clone, Debug, PartialEq)]
pub enum Copied {
    Text(String),
    /// a picture, as PNG
    Image(Vec<u8>),
}

/// The kinds of content taken, in order of preference.
const TEXT: &[&str] = &["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "STRING", "TEXT"];

/// Up to this much of a copy is kept (a bigger one is not history material).
const MAX_BYTES: usize = 8 * 1024 * 1024;

/// Which of an offer's kinds to read (None: a secret, or nothing the history keeps).
pub fn choose(mimes: &[String]) -> Option<String> {
    if mimes.iter().any(|m| m == "x-kde-passwordManagerHint") {
        return None;
    }
    if let Some(m) = mimes.iter().find(|m| *m == "image/png") {
        // (a picture copied as a picture: text kinds next to it are usually its file name)
        if !mimes.iter().any(|m| TEXT.contains(&m.as_str()) && m.starts_with("text/plain")) || mimes.iter().any(|m| m.starts_with("image/")) {
            return Some(m.clone());
        }
    }
    TEXT.iter().find_map(|t| mimes.iter().find(|m| m == t).cloned())
}

struct State {
    seat: Option<wl_seat::WlSeat>,
    manager: Option<manager::ExtDataControlManagerV1>,
    /// each live offer's kinds
    offers: HashMap<u32, Vec<String>>,
    tx: mpsc::Sender<Copied>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(s: &mut Self, r: &wl_registry::WlRegistry, ev: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = ev {
            match interface.as_str() {
                "wl_seat" if s.seat.is_none() => s.seat = Some(r.bind(name, version.min(7), qh, ())),
                "ext_data_control_manager_v1" => s.manager = Some(r.bind(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<manager::ExtDataControlManagerV1, ()> for State {
    fn event(_: &mut Self, _: &manager::ExtDataControlManagerV1, _: manager::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<device::ExtDataControlDeviceV1, ()> for State {
    fn event(s: &mut Self, _: &device::ExtDataControlDeviceV1, ev: device::Event, _: &(), con: &Connection, _: &QueueHandle<Self>) {
        match ev {
            device::Event::DataOffer { id } => {
                s.offers.insert(id.id().protocol_id(), vec![]);
            }
            device::Event::Selection { id: Some(o) } => {
                let mimes = s.offers.get(&o.id().protocol_id()).cloned().unwrap_or_default();
                let Some(mime) = choose(&mimes) else {
                    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() && mimes.iter().any(|m| m == "x-kde-passwordManagerHint") {
                        eprintln!("debug transferencia: um segredo ficou fora do histórico");
                    }
                    return;
                };
                let Ok((mut reader, writer)) = os_pipe::pipe() else { return };
                o.receive(mime.clone(), writer.as_fd());
                drop(writer);
                let _ = con.flush();
                let tx = s.tx.clone();
                std::thread::spawn(move || {
                    let mut data = Vec::new();
                    let _ = (&mut reader).take(MAX_BYTES as u64 + 1).read_to_end(&mut data);
                    if data.is_empty() || data.len() > MAX_BYTES {
                        return;
                    }
                    let c = if mime.starts_with("image/") {
                        Copied::Image(data)
                    } else {
                        Copied::Text(String::from_utf8_lossy(&data).into_owned())
                    };
                    let _ = tx.send(c);
                });
            }
            _ => {}
        }
    }

    event_created_child!(State, device::ExtDataControlDeviceV1, [
        device::EVT_DATA_OFFER_OPCODE => (offer::ExtDataControlOfferV1, ()),
    ]);
}

impl Dispatch<offer::ExtDataControlOfferV1, ()> for State {
    fn event(s: &mut Self, o: &offer::ExtDataControlOfferV1, ev: offer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let offer::Event::Offer { mime_type } = ev {
            s.offers.entry(o.id().protocol_id()).or_default().push(mime_type);
        }
    }
}

/// Starts watching; each new selection comes through `tx`. Err when the compositor has no data control.
pub fn watch(tx: mpsc::Sender<Copied>) -> Result<(), String> {
    let con = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut queue = con.new_event_queue();
        let qh = queue.handle();
        let _registry = con.display().get_registry(&qh, ());
        let mut s = State { seat: None, manager: None, offers: HashMap::new(), tx };
        if queue.roundtrip(&mut s).is_err() {
            let _ = ready_tx.send(Err("roundtrip".to_string()));
            return;
        }
        let (Some(seat), Some(m)) = (s.seat.clone(), s.manager.clone()) else {
            let _ = ready_tx.send(Err("the compositor has no clipboard control".to_string()));
            return;
        };
        let _device = m.get_data_device(&seat, &qh, ());
        let _ = ready_tx.send(Ok(()));
        while queue.blocking_dispatch(&mut s).is_ok() {}
    });
    ready_rx.recv_timeout(std::time::Duration::from_secs(3)).map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_never_and_text_or_pictures_by_preference() {
        let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(choose(&v(&["text/plain", "x-kde-passwordManagerHint", "UTF8_STRING"])), None);
        assert_eq!(choose(&v(&["UTF8_STRING", "text/plain;charset=utf-8", "TEXT"])).as_deref(), Some("text/plain;charset=utf-8"));
        assert_eq!(choose(&v(&["image/png", "image/jpeg"])).as_deref(), Some("image/png"));
        assert_eq!(choose(&v(&["application/x-something"])), None);
    }
}
