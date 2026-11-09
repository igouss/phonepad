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
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn variant_index(state: &Stream) -> usize {
        match state {
            Stream::Hidden => 0,
            Stream::Streaming { .. } => 1,
        }
    }

    fn input_index(input: &StreamInput) -> usize {
        match input {
            StreamInput::Visibility(_) => 0,
            StreamInput::FrameReady => 1,
            StreamInput::Ack => 2,
        }
    }

    fn streaming(in_flight: u8, frame_pending: bool) -> Stream {
        Stream::Streaming {
            in_flight,
            frame_pending,
        }
    }

    fn decision(
        events: Vec<StreamEvent>,
        commands: Vec<StreamCommand>,
    ) -> Decision<StreamEvent, StreamCommand> {
        Decision { events, commands }
    }

    #[test]
    fn all_states_cover_every_variant() {
        assert_eq!(
            all_states()
                .iter()
                .map(variant_index)
                .collect::<BTreeSet<usize>>(),
            BTreeSet::from([0, 1])
        );
    }

    #[test]
    fn all_inputs_cover_every_variant() {
        assert_eq!(
            all_inputs()
                .iter()
                .map(input_index)
                .collect::<BTreeSet<usize>>(),
            BTreeSet::from([0, 1, 2])
        );
    }

    #[test]
    fn all_states_lists_hidden_then_every_reachable_streaming_state() {
        assert_eq!(
            all_states(),
            vec![
                Stream::Hidden,
                streaming(0, false),
                streaming(1, false),
                streaming(2, false),
                streaming(3, false),
                streaming(4, false),
                streaming(4, true),
            ]
        );
    }

    fn reached(inputs: &[StreamInput]) -> Stream {
        inputs
            .iter()
            .fold(Stream::Hidden, |state: Stream, input: &StreamInput| {
                decide(&state, *input).events.iter().fold(state, evolve)
            })
    }

    #[test]
    fn every_listed_state_is_reachable_from_hidden() {
        let shown: StreamInput = StreamInput::Visibility(true);
        let frame: StreamInput = StreamInput::FrameReady;
        assert_eq!(
            all_states(),
            vec![
                reached(&[]),
                reached(&[shown]),
                reached(&[shown, frame]),
                reached(&[shown, frame, frame]),
                reached(&[shown, frame, frame, frame]),
                reached(&[shown, frame, frame, frame, frame]),
                reached(&[shown, frame, frame, frame, frame, frame]),
            ]
        );
    }

    #[test]
    fn all_inputs_lists_every_input() {
        assert_eq!(
            all_inputs(),
            vec![
                StreamInput::Visibility(true),
                StreamInput::Visibility(false),
                StreamInput::FrameReady,
                StreamInput::Ack
            ]
        );
    }

    #[test]
    fn hidden_shown_joins_viewers() {
        assert_eq!(
            decide(&Stream::Hidden, StreamInput::Visibility(true)),
            decision(vec![StreamEvent::Shown], vec![StreamCommand::JoinViewers])
        );
    }

    #[test]
    fn hidden_hidden_does_nothing() {
        assert_eq!(
            decide(&Stream::Hidden, StreamInput::Visibility(false)),
            Decision::none()
        );
    }

    #[test]
    fn streaming_hidden_leaves_viewers() {
        assert_eq!(
            decide(&streaming(2, true), StreamInput::Visibility(false)),
            decision(vec![StreamEvent::Hid], vec![StreamCommand::LeaveViewers])
        );
    }

    #[test]
    fn streaming_shown_does_nothing() {
        assert_eq!(
            decide(&streaming(0, false), StreamInput::Visibility(true)),
            Decision::none()
        );
    }

    #[test]
    fn frame_with_nothing_in_flight_is_sent() {
        assert_eq!(
            decide(&streaming(0, false), StreamInput::FrameReady),
            decision(
                vec![StreamEvent::FrameSent],
                vec![StreamCommand::SendLatestFrame]
            )
        );
    }

    #[test]
    fn frame_below_limit_is_sent() {
        assert_eq!(
            decide(
                &streaming(MAX_IN_FLIGHT - 1, false),
                StreamInput::FrameReady
            ),
            decision(
                vec![StreamEvent::FrameSent],
                vec![StreamCommand::SendLatestFrame]
            )
        );
    }

    #[test]
    fn frame_at_limit_is_deferred() {
        assert_eq!(
            decide(&streaming(MAX_IN_FLIGHT, false), StreamInput::FrameReady),
            decision(vec![StreamEvent::FrameDeferred], vec![])
        );
    }

    #[test]
    fn frame_at_limit_already_pending_does_nothing() {
        assert_eq!(
            decide(&streaming(MAX_IN_FLIGHT, true), StreamInput::FrameReady),
            Decision::none()
        );
    }

    #[test]
    fn frame_while_hidden_does_nothing() {
        assert_eq!(
            decide(&Stream::Hidden, StreamInput::FrameReady),
            Decision::none()
        );
    }

    #[test]
    fn ack_with_pending_frame_sends_it() {
        assert_eq!(
            decide(&streaming(MAX_IN_FLIGHT, true), StreamInput::Ack),
            decision(
                vec![StreamEvent::Acked, StreamEvent::FrameSent],
                vec![StreamCommand::SendLatestFrame]
            )
        );
    }

    #[test]
    fn ack_of_one_frame_with_pending_frame_sends_it() {
        assert_eq!(
            decide(&streaming(1, true), StreamInput::Ack),
            decision(
                vec![StreamEvent::Acked, StreamEvent::FrameSent],
                vec![StreamCommand::SendLatestFrame]
            )
        );
    }

    #[test]
    fn ack_without_pending_frame_only_acks() {
        assert_eq!(
            decide(&streaming(1, false), StreamInput::Ack),
            decision(vec![StreamEvent::Acked], vec![])
        );
    }

    #[test]
    fn ack_with_nothing_in_flight_does_nothing() {
        assert_eq!(
            decide(&streaming(0, false), StreamInput::Ack),
            Decision::none()
        );
    }

    #[test]
    fn ack_with_nothing_in_flight_and_pending_does_nothing() {
        assert_eq!(
            decide(&streaming(0, true), StreamInput::Ack),
            Decision::none()
        );
    }

    #[test]
    fn ack_while_hidden_does_nothing() {
        assert_eq!(decide(&Stream::Hidden, StreamInput::Ack), Decision::none());
    }

    #[test]
    fn shown_starts_streaming_empty() {
        assert_eq!(
            evolve(Stream::Hidden, &StreamEvent::Shown),
            streaming(0, false)
        );
    }

    #[test]
    fn hid_stops_streaming() {
        assert_eq!(
            evolve(streaming(3, true), &StreamEvent::Hid),
            Stream::Hidden
        );
    }

    #[test]
    fn frame_sent_adds_one_in_flight_and_clears_pending() {
        assert_eq!(
            evolve(streaming(3, true), &StreamEvent::FrameSent),
            streaming(4, false)
        );
    }

    #[test]
    fn frame_sent_saturates_in_flight() {
        assert_eq!(
            evolve(streaming(u8::MAX, false), &StreamEvent::FrameSent),
            streaming(u8::MAX, false)
        );
    }

    #[test]
    fn frame_deferred_marks_pending() {
        assert_eq!(
            evolve(streaming(4, false), &StreamEvent::FrameDeferred),
            streaming(4, true)
        );
    }

    #[test]
    fn acked_removes_one_in_flight_and_keeps_pending() {
        assert_eq!(
            evolve(streaming(4, true), &StreamEvent::Acked),
            streaming(3, true)
        );
    }

    #[test]
    fn acked_saturates_at_zero() {
        assert_eq!(
            evolve(streaming(0, false), &StreamEvent::Acked),
            streaming(0, false)
        );
    }

    #[test]
    fn frame_sent_while_hidden_changes_nothing() {
        assert_eq!(
            evolve(Stream::Hidden, &StreamEvent::FrameSent),
            Stream::Hidden
        );
    }

    #[test]
    fn frame_deferred_while_hidden_changes_nothing() {
        assert_eq!(
            evolve(Stream::Hidden, &StreamEvent::FrameDeferred),
            Stream::Hidden
        );
    }

    #[test]
    fn acked_while_hidden_changes_nothing() {
        assert_eq!(evolve(Stream::Hidden, &StreamEvent::Acked), Stream::Hidden);
    }
}
