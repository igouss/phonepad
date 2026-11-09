use super::*;
use std::collections::BTreeSet;

const SHOW: StreamInput = StreamInput::Visibility(true);
const HIDE: StreamInput = StreamInput::Visibility(false);
const KEY: StreamInput = StreamInput::UnitReady(UnitKind::Keyframe);
const DELTA: StreamInput = StreamInput::UnitReady(UnitKind::Delta);
const MISSED: StreamInput = StreamInput::Missed;
const ACK: StreamInput = StreamInput::Ack;

fn unsynced(in_flight: u8) -> Stream {
    Stream::Unsynced {
        in_flight: InFlight(in_flight),
    }
}

fn synced(in_flight: u8) -> Stream {
    Stream::Synced {
        in_flight: InFlight(in_flight),
    }
}

fn one(event: StreamEvent, commands: Vec<StreamCommand>) -> Decision<StreamEvent, StreamCommand> {
    Decision {
        events: vec![event],
        commands,
    }
}

fn step(state: Stream, input: StreamInput) -> Stream {
    decide(&state, input).events.iter().fold(state, evolve)
}

fn reached(inputs: &[StreamInput]) -> Stream {
    inputs.iter().copied().fold(Stream::Hidden, step)
}

fn sent(inputs: &[StreamInput]) -> Vec<StreamCommand> {
    inputs
        .iter()
        .copied()
        .fold(
            (Stream::Hidden, Vec::new()),
            |(state, mut commands): (Stream, Vec<StreamCommand>), input: StreamInput| {
                commands.extend(decide(&state, input).commands);
                (step(state, input), commands)
            },
        )
        .1
}

fn variant_index(state: &Stream) -> usize {
    match state {
        Stream::Hidden => 0,
        Stream::Unsynced { .. } => 1,
        Stream::Synced { .. } => 2,
    }
}

fn input_index(input: &StreamInput) -> usize {
    match input {
        StreamInput::Visibility(true) => 0,
        StreamInput::Visibility(false) => 1,
        StreamInput::UnitReady(UnitKind::Keyframe) => 2,
        StreamInput::UnitReady(UnitKind::Delta) => 3,
        StreamInput::Missed => 4,
        StreamInput::Ack => 5,
    }
}

#[test]
fn a_viewer_never_receives_a_delta_whose_reference_it_did_not_receive() {
    assert_eq!(
        sent(&[
            SHOW, KEY, DELTA, DELTA, DELTA, DELTA, DELTA, ACK, DELTA, ACK, DELTA, KEY, DELTA
        ]),
        vec![
            StreamCommand::JoinViewers,
            StreamCommand::RequestKeyframe,
            StreamCommand::SendUnit,
            StreamCommand::SendUnit,
            StreamCommand::SendUnit,
            StreamCommand::SendUnit,
            StreamCommand::RequestKeyframe,
            StreamCommand::SendUnit,
            StreamCommand::SendUnit,
        ]
    );
}

#[test]
fn a_viewer_that_always_acks_in_time_receives_every_unit() {
    assert_eq!(
        sent(&[
            SHOW, KEY, ACK, DELTA, ACK, DELTA, DELTA, ACK, DELTA, KEY, DELTA
        ]),
        vec![
            StreamCommand::JoinViewers,
            StreamCommand::RequestKeyframe,
            StreamCommand::SendUnit,
            StreamCommand::SendUnit,
            StreamCommand::SendUnit,
            StreamCommand::SendUnit,
            StreamCommand::SendUnit,
            StreamCommand::SendUnit,
            StreamCommand::SendUnit,
        ]
    );
}

#[test]
fn a_keyframe_dropped_at_the_limit_is_requested_once_an_ack_returns_credit() {
    assert_eq!(
        sent(&[SHOW, KEY, DELTA, DELTA, DELTA, DELTA, KEY, ACK, ACK, KEY]),
        vec![
            StreamCommand::JoinViewers,
            StreamCommand::RequestKeyframe,
            StreamCommand::SendUnit,
            StreamCommand::SendUnit,
            StreamCommand::SendUnit,
            StreamCommand::SendUnit,
            StreamCommand::RequestKeyframe,
            StreamCommand::SendUnit,
        ]
    );
}

#[test]
fn a_viewer_that_stops_acking_requests_no_keyframe_however_many_arrive() {
    assert_eq!(
        sent(&[
            SHOW, KEY, DELTA, DELTA, DELTA, DELTA, KEY, DELTA, KEY, KEY, MISSED
        ]),
        vec![
            StreamCommand::JoinViewers,
            StreamCommand::RequestKeyframe,
            StreamCommand::SendUnit,
            StreamCommand::SendUnit,
            StreamCommand::SendUnit,
            StreamCommand::SendUnit,
        ]
    );
}

#[test]
fn a_viewer_that_missed_units_receives_no_delta_until_a_keyframe() {
    assert_eq!(
        sent(&[SHOW, KEY, ACK, DELTA, ACK, MISSED, DELTA, DELTA, KEY, DELTA]),
        vec![
            StreamCommand::JoinViewers,
            StreamCommand::RequestKeyframe,
            StreamCommand::SendUnit,
            StreamCommand::SendUnit,
            StreamCommand::RequestKeyframe,
            StreamCommand::SendUnit,
            StreamCommand::SendUnit,
        ]
    );
}

#[test]
fn all_states_cover_every_variant() {
    assert_eq!(
        all_states()
            .iter()
            .map(variant_index)
            .collect::<BTreeSet<usize>>(),
        BTreeSet::from([0, 1, 2])
    );
}

#[test]
fn all_inputs_cover_every_variant() {
    assert_eq!(
        all_inputs()
            .iter()
            .map(input_index)
            .collect::<BTreeSet<usize>>(),
        BTreeSet::from([0, 1, 2, 3, 4, 5])
    );
}

#[test]
fn all_states_lists_hidden_then_every_credit_unsynced_then_synced() {
    assert_eq!(
        all_states(),
        vec![
            Stream::Hidden,
            unsynced(0),
            unsynced(1),
            unsynced(2),
            unsynced(3),
            unsynced(4),
            synced(0),
            synced(1),
            synced(2),
            synced(3),
            synced(4),
        ]
    );
}

#[test]
fn every_listed_state_is_reachable_from_hidden() {
    assert_eq!(
        all_states(),
        vec![
            reached(&[]),
            reached(&[SHOW, KEY, DELTA, DELTA, DELTA, DELTA, ACK, ACK, ACK, ACK]),
            reached(&[SHOW, KEY, DELTA, DELTA, DELTA, DELTA, ACK, ACK, ACK]),
            reached(&[SHOW, KEY, DELTA, DELTA, DELTA, DELTA, ACK, ACK]),
            reached(&[SHOW, KEY, DELTA, DELTA, DELTA, DELTA, ACK]),
            reached(&[SHOW, KEY, DELTA, DELTA, DELTA, DELTA]),
            reached(&[SHOW, KEY, ACK]),
            reached(&[SHOW, KEY]),
            reached(&[SHOW, KEY, DELTA]),
            reached(&[SHOW, KEY, DELTA, DELTA]),
            reached(&[SHOW, KEY, DELTA, DELTA, DELTA]),
        ]
    );
}

#[test]
fn no_input_leaves_the_listed_states() {
    let listed: Vec<Stream> = all_states();
    let escaped: Vec<Stream> = listed
        .iter()
        .flat_map(|state: &Stream| {
            all_inputs()
                .into_iter()
                .map(move |input: StreamInput| step(*state, input))
        })
        .filter(|target: &Stream| !listed.contains(target))
        .collect();
    assert_eq!(escaped, Vec::<Stream>::new());
}

#[test]
fn all_inputs_lists_every_input() {
    assert_eq!(all_inputs(), vec![SHOW, HIDE, KEY, DELTA, MISSED, ACK]);
}

#[test]
fn hidden_shown_joins_viewers_and_requests_a_keyframe() {
    assert_eq!(
        decide(&Stream::Hidden, SHOW),
        one(
            StreamEvent::Shown,
            vec![StreamCommand::JoinViewers, StreamCommand::RequestKeyframe]
        )
    );
}

#[test]
fn hidden_ignores_every_other_input() {
    assert_eq!(
        [HIDE, KEY, DELTA, MISSED, ACK].map(|input: StreamInput| decide(&Stream::Hidden, input)),
        [
            Decision::none(),
            Decision::none(),
            Decision::none(),
            Decision::none(),
            Decision::none()
        ]
    );
}

#[test]
fn unsynced_hidden_leaves_viewers() {
    assert_eq!(
        decide(&unsynced(2), HIDE),
        one(StreamEvent::Hid, vec![StreamCommand::LeaveViewers])
    );
}

#[test]
fn synced_hidden_leaves_viewers() {
    assert_eq!(
        decide(&synced(4), HIDE),
        one(StreamEvent::Hid, vec![StreamCommand::LeaveViewers])
    );
}

#[test]
fn shown_again_does_nothing() {
    assert_eq!(
        [decide(&unsynced(0), SHOW), decide(&synced(3), SHOW)],
        [Decision::none(), Decision::none()]
    );
}

#[test]
fn unsynced_keyframe_with_no_flight_is_sent() {
    assert_eq!(
        decide(&unsynced(0), KEY),
        one(StreamEvent::KeyframeSent, vec![StreamCommand::SendUnit])
    );
}

#[test]
fn unsynced_keyframe_below_limit_is_sent() {
    assert_eq!(
        decide(&unsynced(MAX_IN_FLIGHT - 1), KEY),
        one(StreamEvent::KeyframeSent, vec![StreamCommand::SendUnit])
    );
}

#[test]
fn unsynced_keyframe_at_limit_is_dropped_without_a_request() {
    assert_eq!(decide(&unsynced(MAX_IN_FLIGHT), KEY), Decision::none());
}

#[test]
fn unsynced_delta_is_never_sent() {
    assert_eq!(
        [
            decide(&unsynced(0), DELTA),
            decide(&unsynced(MAX_IN_FLIGHT), DELTA)
        ],
        [Decision::none(), Decision::none()]
    );
}

#[test]
fn missed_units_while_unsynced_request_a_keyframe_again_only_with_credit() {
    assert_eq!(
        [
            decide(&unsynced(MAX_IN_FLIGHT - 1), MISSED),
            decide(&unsynced(MAX_IN_FLIGHT), MISSED)
        ],
        [
            one(StreamEvent::Desynced, vec![StreamCommand::RequestKeyframe]),
            one(StreamEvent::Desynced, vec![])
        ]
    );
}

#[test]
fn synced_units_below_limit_are_sent() {
    assert_eq!(
        [
            decide(&synced(0), KEY),
            decide(&synced(MAX_IN_FLIGHT - 1), DELTA)
        ],
        [
            one(StreamEvent::KeyframeSent, vec![StreamCommand::SendUnit]),
            one(StreamEvent::DeltaSent, vec![StreamCommand::SendUnit])
        ]
    );
}

#[test]
fn synced_units_at_limit_desync_without_a_request() {
    assert_eq!(
        [
            decide(&synced(MAX_IN_FLIGHT), KEY),
            decide(&synced(MAX_IN_FLIGHT), DELTA)
        ],
        [
            one(StreamEvent::Desynced, vec![]),
            one(StreamEvent::Desynced, vec![])
        ]
    );
}

#[test]
fn missed_units_while_synced_desync_and_request_a_keyframe_only_with_credit() {
    assert_eq!(
        [
            decide(&synced(MAX_IN_FLIGHT - 1), MISSED),
            decide(&synced(MAX_IN_FLIGHT), MISSED)
        ],
        [
            one(StreamEvent::Desynced, vec![StreamCommand::RequestKeyframe]),
            one(StreamEvent::Desynced, vec![])
        ]
    );
}

#[test]
fn ack_with_flight_returns_credit() {
    assert_eq!(
        [
            decide(&unsynced(1), ACK),
            decide(&synced(MAX_IN_FLIGHT), ACK)
        ],
        [
            one(StreamEvent::Acked, vec![]),
            one(StreamEvent::Acked, vec![])
        ]
    );
}

#[test]
fn unsynced_ack_that_restores_credit_requests_a_keyframe() {
    assert_eq!(
        [
            decide(&unsynced(MAX_IN_FLIGHT), ACK),
            decide(&unsynced(MAX_IN_FLIGHT - 1), ACK)
        ],
        [
            one(StreamEvent::Acked, vec![StreamCommand::RequestKeyframe]),
            one(StreamEvent::Acked, vec![])
        ]
    );
}

#[test]
fn ack_with_nothing_in_flight_does_nothing() {
    assert_eq!(
        [decide(&unsynced(0), ACK), decide(&synced(0), ACK)],
        [Decision::none(), Decision::none()]
    );
}

#[test]
fn shown_starts_unsynced_with_nothing_in_flight() {
    assert_eq!(
        [
            evolve(Stream::Hidden, &StreamEvent::Shown),
            evolve(synced(3), &StreamEvent::Shown)
        ],
        [unsynced(0), unsynced(0)]
    );
}

#[test]
fn hid_stops_streaming() {
    assert_eq!(
        [
            evolve(unsynced(2), &StreamEvent::Hid),
            evolve(synced(3), &StreamEvent::Hid)
        ],
        [Stream::Hidden, Stream::Hidden]
    );
}

#[test]
fn keyframe_sent_syncs_and_takes_credit() {
    assert_eq!(
        [
            evolve(unsynced(0), &StreamEvent::KeyframeSent),
            evolve(synced(2), &StreamEvent::KeyframeSent)
        ],
        [synced(1), synced(3)]
    );
}

#[test]
fn delta_sent_takes_credit_and_keeps_sync() {
    assert_eq!(
        [
            evolve(synced(0), &StreamEvent::DeltaSent),
            evolve(synced(3), &StreamEvent::DeltaSent),
            evolve(unsynced(1), &StreamEvent::DeltaSent)
        ],
        [synced(1), synced(4), unsynced(2)]
    );
}

#[test]
fn sent_saturates_at_the_limit() {
    assert_eq!(
        evolve(synced(MAX_IN_FLIGHT), &StreamEvent::DeltaSent),
        synced(MAX_IN_FLIGHT)
    );
}

#[test]
fn desynced_keeps_credit() {
    assert_eq!(
        [
            evolve(synced(4), &StreamEvent::Desynced),
            evolve(unsynced(3), &StreamEvent::Desynced)
        ],
        [unsynced(4), unsynced(3)]
    );
}

#[test]
fn acked_returns_one_credit() {
    assert_eq!(
        [
            evolve(unsynced(4), &StreamEvent::Acked),
            evolve(synced(2), &StreamEvent::Acked)
        ],
        [unsynced(3), synced(1)]
    );
}

#[test]
fn acked_saturates_at_zero() {
    assert_eq!(
        [
            evolve(unsynced(0), &StreamEvent::Acked),
            evolve(synced(0), &StreamEvent::Acked)
        ],
        [unsynced(0), synced(0)]
    );
}

#[test]
fn delivery_events_while_hidden_change_nothing() {
    assert_eq!(
        [
            StreamEvent::KeyframeSent,
            StreamEvent::DeltaSent,
            StreamEvent::Desynced,
            StreamEvent::Acked
        ]
        .map(|event: StreamEvent| evolve(Stream::Hidden, &event)),
        [Stream::Hidden; 4]
    );
}
