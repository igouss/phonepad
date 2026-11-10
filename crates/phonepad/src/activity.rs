use std::thread;

use anyhow::{Context, anyhow};
use phonepad_core::desk::{DeskInput, PHONE_QUIET_MS};
use tokio::sync::mpsc;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{
    self, ExtIdleNotificationV1,
};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::{
    self, ExtIdleNotifierV1,
};

use crate::clock::Clock;
use crate::desk::DeskMessage;

const INPUT_IDLE_VERSION: u32 = 2;

struct Watch {
    desk: mpsc::Sender<DeskMessage>,
    clock: Clock,
    notifier: Option<ExtIdleNotifierV1>,
    seat: Option<WlSeat>,
    desk_gone: bool,
}

pub fn spawn(desk: mpsc::Sender<DeskMessage>, clock: Clock) {
    thread::spawn(move || {
        if let Err(err) = watch(desk, clock) {
            eprintln!(
                "input activity: {err:#}; the pointer stays big until the last viewer leaves"
            );
        }
    });
}

fn watch(desk: mpsc::Sender<DeskMessage>, clock: Clock) -> anyhow::Result<()> {
    let conn: Connection = Connection::connect_to_env().context("wayland connect")?;
    let mut queue: EventQueue<Watch> = conn.new_event_queue();
    let qh: QueueHandle<Watch> = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut state: Watch = Watch {
        desk,
        clock,
        notifier: None,
        seat: None,
        desk_gone: false,
    };
    queue.roundtrip(&mut state).context("wayland registry")?;
    let notifier: &ExtIdleNotifierV1 = state
        .notifier
        .as_ref()
        .ok_or_else(|| anyhow!("no ext_idle_notifier_v1 version {INPUT_IDLE_VERSION}"))?;
    let seat: &WlSeat = state.seat.as_ref().ok_or_else(|| anyhow!("no wl_seat"))?;
    let timeout: u32 = u32::try_from(PHONE_QUIET_MS).context("idle timeout")?;
    notifier.get_input_idle_notification(timeout, seat, &qh, ());
    while !state.desk_gone {
        queue
            .blocking_dispatch(&mut state)
            .context("wayland dispatch")?;
    }
    Ok(())
}

impl Dispatch<WlRegistry, ()> for Watch {
    fn event(
        state: &mut Watch,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Watch>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "ext_idle_notifier_v1" if version >= INPUT_IDLE_VERSION => {
                    state.notifier = Some(registry.bind(name, INPUT_IDLE_VERSION, qh, ()));
                }
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, 1, qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<ExtIdleNotificationV1, ()> for Watch {
    fn event(
        state: &mut Watch,
        _: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Watch>,
    ) {
        if let ext_idle_notification_v1::Event::Resumed = event {
            let input: DeskInput = DeskInput::InputActivity {
                at_ms: state.clock.now_ms(),
            };
            state.desk_gone = state.desk.blocking_send(DeskMessage::Input(input)).is_err();
        }
    }
}

impl Dispatch<ExtIdleNotifierV1, ()> for Watch {
    fn event(
        _: &mut Watch,
        _: &ExtIdleNotifierV1,
        _: ext_idle_notifier_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Watch>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for Watch {
    fn event(
        _: &mut Watch,
        _: &WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Watch>,
    ) {
    }
}
