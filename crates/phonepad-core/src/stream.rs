use crate::decision::Decision;

pub const MAX_IN_FLIGHT: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InFlight(u8);

impl InFlight {
    const NONE: InFlight = InFlight(0);

    fn has_credit(self) -> bool {
        self.0 < MAX_IN_FLIGHT
    }

    fn is_empty(self) -> bool {
        self.0 == 0
    }

    fn sent(self) -> InFlight {
        InFlight(self.0.saturating_add(1).min(MAX_IN_FLIGHT))
    }

    fn acked(self) -> InFlight {
        InFlight(self.0.saturating_sub(1))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Hidden,
    Unsynced { in_flight: InFlight },
    Synced { in_flight: InFlight },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    Keyframe,
    Delta,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamInput {
    Visibility(bool),
    UnitReady(UnitKind),
    Missed,
    Ack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamEvent {
    Shown,
    Hid,
    KeyframeSent,
    DeltaSent,
    Desynced,
    Acked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamCommand {
    JoinViewers,
    LeaveViewers,
    SendUnit,
    RequestKeyframe,
}

pub fn decide(state: &Stream, input: StreamInput) -> Decision<StreamEvent, StreamCommand> {
    match (*state, input) {
        (Stream::Hidden, StreamInput::Visibility(true)) => decision(
            StreamEvent::Shown,
            vec![StreamCommand::JoinViewers, StreamCommand::RequestKeyframe],
        ),
        (Stream::Unsynced { .. } | Stream::Synced { .. }, StreamInput::Visibility(false)) => {
            decision(StreamEvent::Hid, vec![StreamCommand::LeaveViewers])
        }
        (Stream::Unsynced { in_flight }, StreamInput::UnitReady(kind)) => {
            unsynced_unit(in_flight, kind)
        }
        (Stream::Synced { in_flight }, StreamInput::UnitReady(kind)) => {
            synced_unit(in_flight, kind)
        }
        (Stream::Unsynced { in_flight } | Stream::Synced { in_flight }, StreamInput::Missed) => {
            decision(StreamEvent::Desynced, keyframe_if(in_flight.has_credit()))
        }
        (Stream::Unsynced { in_flight }, StreamInput::Ack) if !in_flight.is_empty() => {
            decision(StreamEvent::Acked, keyframe_if(!in_flight.has_credit()))
        }
        (Stream::Synced { in_flight }, StreamInput::Ack) if !in_flight.is_empty() => {
            decision(StreamEvent::Acked, Vec::new())
        }
        (
            Stream::Hidden,
            StreamInput::Visibility(false)
            | StreamInput::UnitReady(_)
            | StreamInput::Missed
            | StreamInput::Ack,
        )
        | (
            Stream::Unsynced { .. } | Stream::Synced { .. },
            StreamInput::Visibility(true) | StreamInput::Ack,
        ) => Decision::none(),
    }
}

fn unsynced_unit(in_flight: InFlight, kind: UnitKind) -> Decision<StreamEvent, StreamCommand> {
    match (kind, in_flight.has_credit()) {
        (UnitKind::Keyframe, true) => {
            decision(StreamEvent::KeyframeSent, vec![StreamCommand::SendUnit])
        }
        (UnitKind::Keyframe, false) | (UnitKind::Delta, true | false) => Decision::none(),
    }
}

fn synced_unit(in_flight: InFlight, kind: UnitKind) -> Decision<StreamEvent, StreamCommand> {
    match (kind, in_flight.has_credit()) {
        (UnitKind::Keyframe, true) => {
            decision(StreamEvent::KeyframeSent, vec![StreamCommand::SendUnit])
        }
        (UnitKind::Delta, true) => decision(StreamEvent::DeltaSent, vec![StreamCommand::SendUnit]),
        (UnitKind::Keyframe | UnitKind::Delta, false) => {
            decision(StreamEvent::Desynced, Vec::new())
        }
    }
}

fn keyframe_if(wanted: bool) -> Vec<StreamCommand> {
    if wanted {
        vec![StreamCommand::RequestKeyframe]
    } else {
        Vec::new()
    }
}

fn decision(
    event: StreamEvent,
    commands: Vec<StreamCommand>,
) -> Decision<StreamEvent, StreamCommand> {
    Decision {
        events: vec![event],
        commands,
    }
}

pub fn evolve(state: Stream, event: &StreamEvent) -> Stream {
    match (state, event) {
        (Stream::Hidden | Stream::Unsynced { .. } | Stream::Synced { .. }, StreamEvent::Shown) => {
            Stream::Unsynced {
                in_flight: InFlight::NONE,
            }
        }
        (Stream::Hidden | Stream::Unsynced { .. } | Stream::Synced { .. }, StreamEvent::Hid) => {
            Stream::Hidden
        }
        (
            Stream::Unsynced { in_flight } | Stream::Synced { in_flight },
            StreamEvent::KeyframeSent,
        ) => Stream::Synced {
            in_flight: in_flight.sent(),
        },
        (Stream::Unsynced { in_flight }, StreamEvent::DeltaSent) => Stream::Unsynced {
            in_flight: in_flight.sent(),
        },
        (Stream::Synced { in_flight }, StreamEvent::DeltaSent) => Stream::Synced {
            in_flight: in_flight.sent(),
        },
        (Stream::Unsynced { in_flight } | Stream::Synced { in_flight }, StreamEvent::Desynced) => {
            Stream::Unsynced { in_flight }
        }
        (Stream::Unsynced { in_flight }, StreamEvent::Acked) => Stream::Unsynced {
            in_flight: in_flight.acked(),
        },
        (Stream::Synced { in_flight }, StreamEvent::Acked) => Stream::Synced {
            in_flight: in_flight.acked(),
        },
        (
            Stream::Hidden,
            StreamEvent::KeyframeSent
            | StreamEvent::DeltaSent
            | StreamEvent::Desynced
            | StreamEvent::Acked,
        ) => Stream::Hidden,
    }
}

pub fn all_states() -> Vec<Stream> {
    std::iter::once(Stream::Hidden)
        .chain(credits().map(|in_flight: InFlight| Stream::Unsynced { in_flight }))
        .chain(credits().map(|in_flight: InFlight| Stream::Synced { in_flight }))
        .collect()
}

fn credits() -> impl Iterator<Item = InFlight> {
    (0..=MAX_IN_FLIGHT).map(InFlight)
}

pub fn all_inputs() -> Vec<StreamInput> {
    vec![
        StreamInput::Visibility(true),
        StreamInput::Visibility(false),
        StreamInput::UnitReady(UnitKind::Keyframe),
        StreamInput::UnitReady(UnitKind::Delta),
        StreamInput::Missed,
        StreamInput::Ack,
    ]
}

#[cfg(test)]
mod tests;
