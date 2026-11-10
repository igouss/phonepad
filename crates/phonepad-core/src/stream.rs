mod pipe;

use crate::decision::Decision;
pub use pipe::{
    CEILING_BYTES, FLOOR_BYTES, MAX_UNITS, MIN_RTT_WINDOW_US, Occupancy, Pipe, QUEUE_CAP_US,
    RATE_WINDOW_US, Sent,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stream {
    Hidden,
    Unsynced { pipe: Pipe },
    Synced { pipe: Pipe },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    Keyframe,
    Delta,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offer {
    pub kind: UnitKind,
    pub unit: Sent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamInput {
    Visibility(bool),
    UnitReady(Offer),
    Missed,
    Ack { id: u32, at_us: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamEvent {
    Shown,
    Hid,
    KeyframeSent(Sent),
    DeltaSent(Sent),
    Desynced,
    Acked { id: u32, at_us: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamCommand {
    JoinViewers,
    LeaveViewers,
    SendUnit,
    RequestKeyframe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Hidden,
    Unsynced(Occupancy),
    Synced(Occupancy),
}

pub fn decide(state: &Stream, input: StreamInput) -> Decision<StreamEvent, StreamCommand> {
    match (state, input) {
        (Stream::Hidden, StreamInput::Visibility(true)) => decision(
            StreamEvent::Shown,
            vec![StreamCommand::JoinViewers, StreamCommand::RequestKeyframe],
        ),
        (Stream::Unsynced { .. } | Stream::Synced { .. }, StreamInput::Visibility(false)) => {
            decision(StreamEvent::Hid, vec![StreamCommand::LeaveViewers])
        }
        (Stream::Unsynced { pipe }, StreamInput::UnitReady(offer)) => unsynced_unit(pipe, offer),
        (Stream::Synced { pipe }, StreamInput::UnitReady(offer)) => synced_unit(pipe, offer),
        (Stream::Unsynced { pipe } | Stream::Synced { pipe }, StreamInput::Missed) => {
            decision(StreamEvent::Desynced, keyframe_if(pipe.is_empty()))
        }
        (Stream::Unsynced { pipe }, StreamInput::Ack { id, at_us }) if pipe.carries(id) => {
            decision(
                StreamEvent::Acked { id, at_us },
                keyframe_if(pipe.ends_with(id)),
            )
        }
        (Stream::Synced { pipe }, StreamInput::Ack { id, at_us }) if pipe.carries(id) => {
            decision(StreamEvent::Acked { id, at_us }, Vec::new())
        }
        (
            Stream::Hidden,
            StreamInput::Visibility(false)
            | StreamInput::UnitReady(_)
            | StreamInput::Missed
            | StreamInput::Ack { .. },
        )
        | (
            Stream::Unsynced { .. } | Stream::Synced { .. },
            StreamInput::Visibility(true) | StreamInput::Ack { .. },
        ) => Decision::none(),
    }
}

fn unsynced_unit(pipe: &Pipe, offer: Offer) -> Decision<StreamEvent, StreamCommand> {
    match (offer.kind, pipe.admits(offer.unit.bytes)) {
        (UnitKind::Keyframe, true) => decision(
            StreamEvent::KeyframeSent(offer.unit),
            vec![StreamCommand::SendUnit],
        ),
        (UnitKind::Keyframe, false) | (UnitKind::Delta, true | false) => Decision::none(),
    }
}

fn synced_unit(pipe: &Pipe, offer: Offer) -> Decision<StreamEvent, StreamCommand> {
    match (offer.kind, pipe.admits(offer.unit.bytes)) {
        (UnitKind::Keyframe, true) => decision(
            StreamEvent::KeyframeSent(offer.unit),
            vec![StreamCommand::SendUnit],
        ),
        (UnitKind::Delta, true) => decision(
            StreamEvent::DeltaSent(offer.unit),
            vec![StreamCommand::SendUnit],
        ),
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
    match (state, *event) {
        (Stream::Hidden | Stream::Unsynced { .. } | Stream::Synced { .. }, StreamEvent::Shown) => {
            Stream::Unsynced {
                pipe: Pipe::default(),
            }
        }
        (Stream::Hidden | Stream::Unsynced { .. } | Stream::Synced { .. }, StreamEvent::Hid) => {
            Stream::Hidden
        }
        (Stream::Unsynced { pipe } | Stream::Synced { pipe }, StreamEvent::KeyframeSent(sent)) => {
            Stream::Synced {
                pipe: pipe.sent(sent),
            }
        }
        (Stream::Unsynced { pipe }, StreamEvent::DeltaSent(sent)) => Stream::Unsynced {
            pipe: pipe.sent(sent),
        },
        (Stream::Synced { pipe }, StreamEvent::DeltaSent(sent)) => Stream::Synced {
            pipe: pipe.sent(sent),
        },
        (Stream::Unsynced { pipe } | Stream::Synced { pipe }, StreamEvent::Desynced) => {
            Stream::Unsynced { pipe }
        }
        (Stream::Unsynced { pipe }, StreamEvent::Acked { id, at_us }) => Stream::Unsynced {
            pipe: pipe.acked(id, at_us),
        },
        (Stream::Synced { pipe }, StreamEvent::Acked { id, at_us }) => Stream::Synced {
            pipe: pipe.acked(id, at_us),
        },
        (
            Stream::Hidden,
            StreamEvent::KeyframeSent(_)
            | StreamEvent::DeltaSent(_)
            | StreamEvent::Desynced
            | StreamEvent::Acked { .. },
        ) => Stream::Hidden,
    }
}

pub fn phase(state: &Stream) -> Phase {
    match state {
        Stream::Hidden => Phase::Hidden,
        Stream::Unsynced { pipe } => Phase::Unsynced(pipe.occupancy()),
        Stream::Synced { pipe } => Phase::Synced(pipe.occupancy()),
    }
}

const SAMPLE_BYTES: u32 = 1000;

fn sample_sent(id: u32, bytes: u32) -> Sent {
    Sent {
        id,
        bytes,
        at_us: u64::from(id) * 1000,
    }
}

fn sample_pipes() -> [Pipe; 3] {
    let big: u32 = u32::try_from(FLOOR_BYTES).unwrap_or(u32::MAX);
    [
        Pipe::default(),
        Pipe::default()
            .sent(sample_sent(1, SAMPLE_BYTES))
            .sent(sample_sent(2, SAMPLE_BYTES)),
        Pipe::default()
            .sent(sample_sent(1, big))
            .sent(sample_sent(2, SAMPLE_BYTES)),
    ]
}

pub fn all_states() -> Vec<Stream> {
    std::iter::once(Stream::Hidden)
        .chain(sample_pipes().map(|pipe: Pipe| Stream::Unsynced { pipe }))
        .chain(sample_pipes().map(|pipe: Pipe| Stream::Synced { pipe }))
        .collect()
}

pub fn all_inputs() -> Vec<StreamInput> {
    vec![
        StreamInput::Visibility(true),
        StreamInput::Visibility(false),
        StreamInput::UnitReady(Offer {
            kind: UnitKind::Keyframe,
            unit: sample_sent(3, SAMPLE_BYTES),
        }),
        StreamInput::UnitReady(Offer {
            kind: UnitKind::Delta,
            unit: sample_sent(3, SAMPLE_BYTES),
        }),
        StreamInput::Missed,
        StreamInput::Ack {
            id: 1,
            at_us: 50_000,
        },
        StreamInput::Ack {
            id: 2,
            at_us: 50_000,
        },
    ]
}

#[cfg(test)]
mod tests;
