use crate::decision::Decision;

pub const MAX_IN_FLIGHT: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Hidden,
    Streaming { in_flight: u8, frame_pending: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamInput {
    Visibility(bool),
    FrameReady,
    Ack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamEvent {
    Shown,
    Hid,
    FrameSent,
    FrameDeferred,
    Acked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamCommand {
    JoinViewers,
    LeaveViewers,
    SendLatestFrame,
}

pub fn decide(state: &Stream, input: StreamInput) -> Decision<StreamEvent, StreamCommand> {
    match (*state, input) {
        (Stream::Hidden, StreamInput::Visibility(true)) => Decision {
            events: vec![StreamEvent::Shown],
            commands: vec![StreamCommand::JoinViewers],
        },
        (Stream::Streaming { .. }, StreamInput::Visibility(false)) => Decision {
            events: vec![StreamEvent::Hid],
            commands: vec![StreamCommand::LeaveViewers],
        },
        (Stream::Streaming { in_flight, .. }, StreamInput::FrameReady)
            if in_flight < MAX_IN_FLIGHT =>
        {
            Decision {
                events: vec![StreamEvent::FrameSent],
                commands: vec![StreamCommand::SendLatestFrame],
            }
        }
        (
            Stream::Streaming {
                frame_pending: false,
                ..
            },
            StreamInput::FrameReady,
        ) => Decision {
            events: vec![StreamEvent::FrameDeferred],
            commands: Vec::new(),
        },
        (
            Stream::Streaming {
                in_flight,
                frame_pending: true,
            },
            StreamInput::Ack,
        ) if in_flight > 0 => Decision {
            events: vec![StreamEvent::Acked, StreamEvent::FrameSent],
            commands: vec![StreamCommand::SendLatestFrame],
        },
        (
            Stream::Streaming {
                in_flight,
                frame_pending: false,
            },
            StreamInput::Ack,
        ) if in_flight > 0 => Decision {
            events: vec![StreamEvent::Acked],
            commands: Vec::new(),
        },
        (
            Stream::Hidden,
            StreamInput::Visibility(false) | StreamInput::FrameReady | StreamInput::Ack,
        )
        | (
            Stream::Streaming { .. },
            StreamInput::Visibility(true) | StreamInput::FrameReady | StreamInput::Ack,
        ) => Decision::none(),
    }
}

pub fn evolve(state: Stream, event: &StreamEvent) -> Stream {
    match (state, event) {
        (Stream::Hidden | Stream::Streaming { .. }, StreamEvent::Shown) => Stream::Streaming {
            in_flight: 0,
            frame_pending: false,
        },
        (Stream::Hidden | Stream::Streaming { .. }, StreamEvent::Hid) => Stream::Hidden,
        (Stream::Streaming { in_flight, .. }, StreamEvent::FrameSent) => Stream::Streaming {
            in_flight: in_flight.saturating_add(1),
            frame_pending: false,
        },
        (Stream::Streaming { in_flight, .. }, StreamEvent::FrameDeferred) => Stream::Streaming {
            in_flight,
            frame_pending: true,
        },
        (
            Stream::Streaming {
                in_flight,
                frame_pending,
            },
            StreamEvent::Acked,
        ) => Stream::Streaming {
            in_flight: in_flight.saturating_sub(1),
            frame_pending,
        },
        (
            Stream::Hidden,
            StreamEvent::FrameSent | StreamEvent::FrameDeferred | StreamEvent::Acked,
        ) => Stream::Hidden,
    }
}

pub fn all_states() -> Vec<Stream> {
    std::iter::once(Stream::Hidden)
        .chain((0..=MAX_IN_FLIGHT).map(|in_flight: u8| Stream::Streaming {
            in_flight,
            frame_pending: false,
        }))
        .chain(std::iter::once(Stream::Streaming {
            in_flight: MAX_IN_FLIGHT,
            frame_pending: true,
        }))
        .collect()
}

pub fn all_inputs() -> Vec<StreamInput> {
    vec![
        StreamInput::Visibility(true),
        StreamInput::Visibility(false),
        StreamInput::FrameReady,
        StreamInput::Ack,
    ]
}

#[cfg(test)]
mod tests;
