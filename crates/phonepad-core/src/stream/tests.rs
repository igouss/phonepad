use super::*;

const SHOW: StreamInput = StreamInput::Visibility(true);
const HIDE: StreamInput = StreamInput::Visibility(false);
const MISSED: StreamInput = StreamInput::Missed;
const SMALL: u32 = 1000;
const BIG: u32 = 64 * 1024;

fn sent(id: u32, bytes: u32) -> Sent {
    Sent {
        id,
        bytes,
        at_us: u64::from(id) * 1000,
    }
}

fn offer(kind: UnitKind, id: u32, bytes: u32) -> StreamInput {
    StreamInput::UnitReady(Offer {
        kind,
        unit: sent(id, bytes),
    })
}

fn key(id: u32) -> StreamInput {
    offer(UnitKind::Keyframe, id, SMALL)
}

fn big_key(id: u32) -> StreamInput {
    offer(UnitKind::Keyframe, id, BIG)
}

fn delta(id: u32) -> StreamInput {
    offer(UnitKind::Delta, id, SMALL)
}

fn big_delta(id: u32) -> StreamInput {
    offer(UnitKind::Delta, id, BIG)
}

fn ack(id: u32) -> StreamInput {
    StreamInput::Ack {
        id,
        at_us: 1_000_000 + u64::from(id),
    }
}

fn flying(ids: &[(u32, u32)]) -> Pipe {
    ids.iter()
        .fold(Pipe::default(), |pipe: Pipe, (id, bytes): &(u32, u32)| {
            pipe.sent(sent(*id, *bytes))
        })
}

fn unsynced(ids: &[(u32, u32)]) -> Stream {
    Stream::Unsynced { pipe: flying(ids) }
}

fn synced(ids: &[(u32, u32)]) -> Stream {
    Stream::Synced { pipe: flying(ids) }
}

fn one(event: StreamEvent, commands: Vec<StreamCommand>) -> Decision<StreamEvent, StreamCommand> {
    Decision {
        events: vec![event],
        commands,
    }
}

fn step(state: Stream, input: StreamInput) -> Stream {
    let decision: Decision<StreamEvent, StreamCommand> = decide(&state, input);
    decision.events.iter().fold(state, evolve)
}

fn reached(inputs: &[StreamInput]) -> Phase {
    phase(&inputs.iter().copied().fold(Stream::Hidden, step))
}

fn commands(inputs: &[StreamInput]) -> Vec<StreamCommand> {
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

const JOIN: StreamCommand = StreamCommand::JoinViewers;
const SEND: StreamCommand = StreamCommand::SendUnit;
const REQUEST: StreamCommand = StreamCommand::RequestKeyframe;

#[test]
fn a_viewer_never_receives_a_delta_whose_reference_it_did_not_receive() {
    assert_eq!(
        commands(&[
            SHOW,
            delta(1),
            key(2),
            big_delta(3),
            delta(4),
            ack(2),
            delta(5),
            key(6),
            delta(7)
        ]),
        vec![JOIN, REQUEST, SEND, REQUEST, SEND, SEND]
    );
}

#[test]
fn a_viewer_that_always_acks_in_time_receives_every_unit() {
    assert_eq!(
        commands(&[
            SHOW,
            key(1),
            ack(1),
            delta(2),
            ack(2),
            delta(3),
            delta(4),
            ack(3),
            delta(5),
            key(6),
            delta(7)
        ]),
        vec![JOIN, REQUEST, SEND, SEND, SEND, SEND, SEND, SEND, SEND]
    );
}

#[test]
fn a_viewer_takes_more_than_four_units_unacknowledged_while_they_fit_the_window() {
    assert_eq!(
        commands(&[
            SHOW,
            key(1),
            delta(2),
            delta(3),
            delta(4),
            delta(5),
            delta(6),
            delta(7),
            delta(8)
        ]),
        vec![
            JOIN, REQUEST, SEND, SEND, SEND, SEND, SEND, SEND, SEND, SEND
        ]
    );
}

#[test]
fn a_keyframe_refused_for_want_of_room_is_requested_once_the_flight_empties() {
    assert_eq!(
        commands(&[SHOW, big_key(1), delta(2), key(3), ack(1), key(4)]),
        vec![JOIN, REQUEST, SEND, REQUEST, SEND]
    );
}

#[test]
fn a_viewer_that_stops_acking_requests_no_keyframe_however_many_arrive() {
    assert_eq!(
        commands(&[SHOW, big_key(1), delta(2), key(3), delta(4), key(5), MISSED]),
        vec![JOIN, REQUEST, SEND]
    );
}

#[test]
fn a_viewer_that_missed_units_receives_no_delta_until_a_keyframe() {
    assert_eq!(
        commands(&[
            SHOW,
            key(1),
            ack(1),
            delta(2),
            ack(2),
            MISSED,
            delta(3),
            delta(4),
            key(5),
            delta(6)
        ]),
        vec![JOIN, REQUEST, SEND, SEND, REQUEST, SEND, SEND]
    );
}

#[test]
fn a_viewer_that_missed_units_with_units_in_flight_asks_when_the_last_one_lands() {
    assert_eq!(
        commands(&[SHOW, key(1), delta(2), MISSED, ack(1), ack(2)]),
        vec![JOIN, REQUEST, SEND, SEND, REQUEST]
    );
}

#[test]
fn one_ack_lands_every_unit_sent_before_it() {
    assert_eq!(
        commands(&[SHOW, key(1), delta(2), delta(3), MISSED, ack(3), ack(1)]),
        vec![JOIN, REQUEST, SEND, SEND, SEND, REQUEST]
    );
}

#[test]
fn all_states_cover_every_phase() {
    assert_eq!(
        all_states().iter().map(phase).collect::<Vec<Phase>>(),
        vec![
            Phase::Hidden,
            Phase::Unsynced(Occupancy::Empty),
            Phase::Unsynced(Occupancy::Room),
            Phase::Unsynced(Occupancy::Full),
            Phase::Synced(Occupancy::Empty),
            Phase::Synced(Occupancy::Room),
            Phase::Synced(Occupancy::Full),
        ]
    );
}

#[test]
fn every_listed_phase_is_reachable_from_hidden() {
    assert_eq!(
        all_states().iter().map(phase).collect::<Vec<Phase>>(),
        vec![
            reached(&[]),
            reached(&[SHOW]),
            reached(&[SHOW, key(1), delta(2), MISSED]),
            reached(&[SHOW, big_key(1), big_delta(2)]),
            reached(&[SHOW, key(1), ack(1)]),
            reached(&[SHOW, key(1)]),
            reached(&[SHOW, big_key(1)]),
        ]
    );
}

#[test]
fn no_input_leaves_the_listed_phases() {
    let listed: Vec<Phase> = all_states().iter().map(phase).collect();
    let escaped: Vec<Phase> = all_states()
        .into_iter()
        .flat_map(|state: Stream| {
            all_inputs()
                .into_iter()
                .map(move |input: StreamInput| phase(&step(state.clone(), input)))
        })
        .filter(|target: &Phase| !listed.contains(target))
        .collect();
    assert_eq!(escaped, Vec::<Phase>::new());
}

#[test]
fn all_inputs_lists_every_input() {
    assert_eq!(
        all_inputs(),
        vec![
            SHOW,
            HIDE,
            offer(UnitKind::Keyframe, 3, SMALL),
            offer(UnitKind::Delta, 3, SMALL),
            MISSED,
            StreamInput::Ack {
                id: 1,
                at_us: 50_000
            },
            StreamInput::Ack {
                id: 2,
                at_us: 50_000
            },
        ]
    );
}

#[test]
fn the_listed_inputs_move_the_room_phase_to_every_neighbour() {
    let room: Stream = synced(&[(1, SMALL), (2, SMALL)]);
    assert_eq!(
        all_inputs()
            .into_iter()
            .map(|input: StreamInput| phase(&step(room.clone(), input)))
            .collect::<Vec<Phase>>(),
        vec![
            Phase::Synced(Occupancy::Room),
            Phase::Hidden,
            Phase::Synced(Occupancy::Room),
            Phase::Synced(Occupancy::Room),
            Phase::Unsynced(Occupancy::Room),
            Phase::Synced(Occupancy::Room),
            Phase::Synced(Occupancy::Empty),
        ]
    );
}

#[test]
fn hidden_shown_joins_viewers_and_requests_a_keyframe() {
    assert_eq!(
        decide(&Stream::Hidden, SHOW),
        one(StreamEvent::Shown, vec![JOIN, REQUEST])
    );
}

#[test]
fn hidden_ignores_every_other_input() {
    assert_eq!(
        [HIDE, key(1), delta(1), MISSED, ack(1)]
            .map(|input: StreamInput| decide(&Stream::Hidden, input)),
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
        decide(&unsynced(&[(1, SMALL)]), HIDE),
        one(StreamEvent::Hid, vec![StreamCommand::LeaveViewers])
    );
}

#[test]
fn synced_hidden_leaves_viewers() {
    assert_eq!(
        decide(&synced(&[]), HIDE),
        one(StreamEvent::Hid, vec![StreamCommand::LeaveViewers])
    );
}

#[test]
fn shown_again_does_nothing() {
    assert_eq!(
        [
            decide(&unsynced(&[]), SHOW),
            decide(&synced(&[(1, SMALL)]), SHOW)
        ],
        [Decision::none(), Decision::none()]
    );
}

#[test]
fn unsynced_keyframe_into_an_empty_pipe_is_sent_however_big() {
    assert_eq!(
        decide(&unsynced(&[]), offer(UnitKind::Keyframe, 9, u32::MAX)),
        one(StreamEvent::KeyframeSent(sent(9, u32::MAX)), vec![SEND])
    );
}

#[test]
fn unsynced_keyframe_with_room_is_sent() {
    assert_eq!(
        decide(&unsynced(&[(1, SMALL)]), key(2)),
        one(StreamEvent::KeyframeSent(sent(2, SMALL)), vec![SEND])
    );
}

#[test]
fn unsynced_keyframe_without_room_is_dropped_without_a_request() {
    assert_eq!(decide(&unsynced(&[(1, BIG)]), key(2)), Decision::none());
}

#[test]
fn unsynced_delta_is_never_sent() {
    assert_eq!(
        [
            decide(&unsynced(&[]), delta(1)),
            decide(&unsynced(&[(1, BIG)]), delta(2))
        ],
        [Decision::none(), Decision::none()]
    );
}

#[test]
fn missed_units_while_unsynced_request_a_keyframe_only_with_an_empty_flight() {
    assert_eq!(
        [
            decide(&unsynced(&[]), MISSED),
            decide(&unsynced(&[(1, SMALL)]), MISSED)
        ],
        [
            one(StreamEvent::Desynced, vec![REQUEST]),
            one(StreamEvent::Desynced, vec![])
        ]
    );
}

#[test]
fn synced_units_with_room_are_sent() {
    assert_eq!(
        [
            decide(&synced(&[]), key(1)),
            decide(&synced(&[(1, SMALL)]), delta(2))
        ],
        [
            one(StreamEvent::KeyframeSent(sent(1, SMALL)), vec![SEND]),
            one(StreamEvent::DeltaSent(sent(2, SMALL)), vec![SEND])
        ]
    );
}

#[test]
fn synced_delta_into_an_empty_pipe_is_sent_however_big() {
    assert_eq!(
        decide(&synced(&[]), offer(UnitKind::Delta, 9, u32::MAX)),
        one(StreamEvent::DeltaSent(sent(9, u32::MAX)), vec![SEND])
    );
}

#[test]
fn synced_units_without_room_desync_without_a_request() {
    assert_eq!(
        [
            decide(&synced(&[(1, BIG)]), key(2)),
            decide(&synced(&[(1, BIG)]), delta(2))
        ],
        [
            one(StreamEvent::Desynced, vec![]),
            one(StreamEvent::Desynced, vec![])
        ]
    );
}

#[test]
fn missed_units_while_synced_desync_and_request_a_keyframe_only_with_an_empty_flight() {
    assert_eq!(
        [
            decide(&synced(&[]), MISSED),
            decide(&synced(&[(1, SMALL)]), MISSED)
        ],
        [
            one(StreamEvent::Desynced, vec![REQUEST]),
            one(StreamEvent::Desynced, vec![])
        ]
    );
}

#[test]
fn an_ack_for_a_unit_in_flight_lands_it() {
    assert_eq!(
        [
            decide(&unsynced(&[(1, SMALL), (2, SMALL)]), ack(1)),
            decide(&synced(&[(1, SMALL)]), ack(1))
        ],
        [
            one(
                StreamEvent::Acked {
                    id: 1,
                    at_us: 1_000_001
                },
                vec![]
            ),
            one(
                StreamEvent::Acked {
                    id: 1,
                    at_us: 1_000_001
                },
                vec![]
            )
        ]
    );
}

#[test]
fn an_unsynced_ack_that_empties_the_flight_requests_a_keyframe() {
    assert_eq!(
        decide(&unsynced(&[(1, SMALL), (2, SMALL)]), ack(2)),
        one(
            StreamEvent::Acked {
                id: 2,
                at_us: 1_000_002
            },
            vec![REQUEST]
        )
    );
}

#[test]
fn an_ack_for_a_unit_not_in_flight_does_nothing() {
    assert_eq!(
        [
            decide(&unsynced(&[]), ack(1)),
            decide(&synced(&[]), ack(1)),
            decide(&unsynced(&[(2, SMALL)]), ack(1)),
            decide(&synced(&[(2, SMALL)]), ack(1))
        ],
        [
            Decision::none(),
            Decision::none(),
            Decision::none(),
            Decision::none()
        ]
    );
}

#[test]
fn shown_starts_unsynced_with_an_empty_pipe() {
    assert_eq!(
        [
            evolve(Stream::Hidden, &StreamEvent::Shown),
            evolve(synced(&[(1, SMALL)]), &StreamEvent::Shown)
        ],
        [unsynced(&[]), unsynced(&[])]
    );
}

#[test]
fn hid_stops_streaming() {
    assert_eq!(
        [
            evolve(unsynced(&[(1, SMALL)]), &StreamEvent::Hid),
            evolve(synced(&[]), &StreamEvent::Hid)
        ],
        [Stream::Hidden, Stream::Hidden]
    );
}

#[test]
fn keyframe_sent_syncs_and_enters_the_pipe() {
    assert_eq!(
        [
            evolve(unsynced(&[]), &StreamEvent::KeyframeSent(sent(1, SMALL))),
            evolve(
                synced(&[(1, SMALL)]),
                &StreamEvent::KeyframeSent(sent(2, BIG))
            )
        ],
        [synced(&[(1, SMALL)]), synced(&[(1, SMALL), (2, BIG)])]
    );
}

#[test]
fn delta_sent_enters_the_pipe_and_keeps_sync() {
    assert_eq!(
        [
            evolve(synced(&[]), &StreamEvent::DeltaSent(sent(1, SMALL))),
            evolve(
                unsynced(&[(1, SMALL)]),
                &StreamEvent::DeltaSent(sent(2, SMALL))
            )
        ],
        [synced(&[(1, SMALL)]), unsynced(&[(1, SMALL), (2, SMALL)])]
    );
}

#[test]
fn desynced_keeps_the_pipe() {
    assert_eq!(
        [
            evolve(synced(&[(1, SMALL)]), &StreamEvent::Desynced),
            evolve(unsynced(&[(1, BIG)]), &StreamEvent::Desynced)
        ],
        [unsynced(&[(1, SMALL)]), unsynced(&[(1, BIG)])]
    );
}

#[test]
fn acked_lands_the_unit_in_the_pipe() {
    let at: StreamEvent = StreamEvent::Acked {
        id: 1,
        at_us: 9_000,
    };
    assert_eq!(
        [
            evolve(unsynced(&[(1, SMALL), (2, SMALL)]), &at),
            evolve(synced(&[(1, SMALL)]), &at)
        ],
        [
            Stream::Unsynced {
                pipe: flying(&[(1, SMALL), (2, SMALL)]).acked(1, 9_000)
            },
            Stream::Synced {
                pipe: flying(&[(1, SMALL)]).acked(1, 9_000)
            }
        ]
    );
}

#[test]
fn delivery_events_while_hidden_change_nothing() {
    assert_eq!(
        [
            StreamEvent::KeyframeSent(sent(1, SMALL)),
            StreamEvent::DeltaSent(sent(1, SMALL)),
            StreamEvent::Desynced,
            StreamEvent::Acked { id: 1, at_us: 0 }
        ]
        .map(|event: StreamEvent| evolve(Stream::Hidden, &event)),
        [
            Stream::Hidden,
            Stream::Hidden,
            Stream::Hidden,
            Stream::Hidden
        ]
    );
}
