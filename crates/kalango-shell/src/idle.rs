//! When the person stops using the computer (the compositor's ext-idle-notify-v1): after the minutes chosen in
//! Settings › Power the screens go dark (niri powers them off; any input brings them back), and after more minutes
//! the computer suspends (logind) — unless an agent is working and the person asked the agents to keep it awake.
//!
//! A Wayland connection of its own, in a thread: the compositor tells it when the seat has been idle for each time,
//! and when it is used again.

use std::sync::mpsc;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::{ext_idle_notification_v1, ext_idle_notifier_v1};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IdleEvent {
    /// idle for the time of this id
    Idle(u32),
    /// used again
    Resumed(u32),
}

struct State {
    seat: Option<wl_seat::WlSeat>,
    notifier: Option<ext_idle_notifier_v1::ExtIdleNotifierV1>,
    tx: mpsc::Sender<(u64, IdleEvent)>,
    generation: u64,
    /// a newer watch took the receiver: this one ends at its next event
    done: bool,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(e: &mut Self, r: &wl_registry::WlRegistry, ev: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = ev {
            match interface.as_str() {
                "wl_seat" if e.seat.is_none() => e.seat = Some(r.bind(name, version.min(7), qh, ())),
                "ext_idle_notifier_v1" => e.notifier = Some(r.bind(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ext_idle_notifier_v1::ExtIdleNotifierV1, ()> for State {
    fn event(_: &mut Self, _: &ext_idle_notifier_v1::ExtIdleNotifierV1, _: ext_idle_notifier_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, u32> for State {
    fn event(e: &mut Self, _: &ext_idle_notification_v1::ExtIdleNotificationV1, ev: ext_idle_notification_v1::Event, id: &u32, _: &Connection, _: &QueueHandle<Self>) {
        let ev = match ev {
            ext_idle_notification_v1::Event::Idled => IdleEvent::Idle(*id),
            ext_idle_notification_v1::Event::Resumed => IdleEvent::Resumed(*id),
            _ => return,
        };
        e.done = e.tx.send((e.generation, ev)).is_err();
    }
}

/// Starts watching these times (id, milliseconds); the events come through `tx` with `geracao` (a newer watch makes
/// an older one's events stale: the caller drops them). Err when the compositor has no idle notifications.
pub fn watch(timeouts: Vec<(u32, u32)>, generation: u64, tx: mpsc::Sender<(u64, IdleEvent)>) -> Result<(), String> {
    let con = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut queue = con.new_event_queue();
        let qh = queue.handle();
        let _registry = con.display().get_registry(&qh, ());
        let mut e = State { seat: None, notifier: None, tx, generation, done: false };
        if queue.roundtrip(&mut e).is_err() {
            let _ = ready_tx.send(Err("roundtrip".to_string()));
            return;
        }
        let (Some(seat), Some(n)) = (e.seat.clone(), e.notifier.clone()) else {
            let _ = ready_tx.send(Err("the compositor has no idle notifications".to_string()));
            return;
        };
        let _notifications: Vec<_> = timeouts.iter().filter(|(_, ms)| *ms > 0).map(|(id, ms)| n.get_idle_notification(*ms, &seat, &qh, *id)).collect();
        let _ = ready_tx.send(Ok(()));
        // until the shell ends, or a newer watch replaced this one (its notifications and connection go with it)
        while !e.done && queue.blocking_dispatch(&mut e).is_ok() {}
    });
    ready_rx.recv_timeout(std::time::Duration::from_secs(3)).map_err(|e| e.to_string())?
}
