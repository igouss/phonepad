use super::*;

fn sent(id: u32) -> Sent {
    Sent {
        id,
        kind: UnitKind::Delta,
        bytes: 100 + id as usize,
        captured_ns: 1_000_000,
        sent_ns: 3_500_000,
    }
}

fn ledger(ids: std::ops::Range<u32>) -> Ledger {
    ids.map(sent)
        .fold(Ledger::default(), |mut ledger: Ledger, s: Sent| {
            ledger.sent(s);
            ledger
        })
}

fn unit(id: u32, key: bool, capture_send_ms: f64, send_ack_ms: f64) -> Option<Record> {
    Some(Record::Unit {
        id,
        key,
        bytes: 100 + id as usize,
        capture_send_ms,
        send_ack_ms,
    })
}

fn drawn(id: u32, received_ms: f64, decoded_ms: f64, drawn_ms: f64) -> Drawn {
    Drawn {
        id,
        received_ms,
        decoded_ms,
        drawn_ms,
    }
}

#[test]
fn an_ack_for_no_sent_unit_is_no_record() {
    assert_eq!(Ledger::default().acked(7, 9_000_000), None);
}

#[test]
fn one_acked_delta_times_capture_to_send_and_send_to_ack() {
    assert_eq!(ledger(4..5).acked(4, 63_500_000), unit(4, false, 2.5, 60.0));
}

#[test]
fn an_acked_keyframe_is_a_key_record() {
    let mut keyed: Ledger = Ledger::default();
    keyed.sent(Sent {
        kind: UnitKind::Keyframe,
        ..sent(1)
    });
    assert_eq!(keyed.acked(1, 3_500_000), unit(1, true, 2.5, 0.0));
}

#[test]
fn a_unit_is_acked_once() {
    let mut once: Ledger = ledger(4..5);
    once.acked(4, 63_500_000);
    assert_eq!(once.acked(4, 63_500_000), None);
}

#[test]
fn many_sent_units_are_acked_by_their_own_id() {
    let mut many: Ledger = ledger(1..4);
    assert_eq!(
        [many.acked(2, 4_500_000), many.acked(1, 5_500_000)],
        [unit(2, false, 2.5, 1.0), unit(1, false, 2.5, 2.0)]
    );
}

#[test]
fn an_ack_before_the_send_instant_reads_as_zero() {
    assert_eq!(ledger(4..5).acked(4, 0), unit(4, false, 2.5, 0.0));
}

#[test]
fn a_capture_after_the_send_instant_reads_as_zero() {
    let mut late: Ledger = Ledger::default();
    late.sent(Sent {
        captured_ns: 9_000_000,
        ..sent(1)
    });
    assert_eq!(late.acked(1, 3_500_000), unit(1, false, 0.0, 0.0));
}

#[test]
fn the_ledger_keeps_the_latest_units_only() {
    let mut full: Ledger = ledger(0..(KEPT as u32 + 1));
    assert_eq!(
        [full.acked(0, 3_500_000), full.acked(1, 3_500_000)],
        [None, unit(1, false, 2.5, 0.0)]
    );
}

#[test]
fn a_ledger_at_capacity_keeps_its_oldest_unit() {
    let mut full: Ledger = ledger(0..(KEPT as u32));
    assert_eq!(full.acked(0, 3_500_000), unit(0, false, 2.5, 0.0));
}

#[test]
fn the_first_drawn_unit_has_no_gap() {
    assert_eq!(
        Ledger::default().drawn(drawn(3, 100.0, 104.5, 110.0)),
        Record::Drawn {
            id: 3,
            decode_ms: 4.5,
            draw_ms: 5.5,
            gap_ms: None
        }
    );
}

#[test]
fn a_later_drawn_unit_has_the_gap_since_the_previous_draw() {
    let mut drawing: Ledger = Ledger::default();
    drawing.drawn(drawn(3, 100.0, 104.5, 110.0));
    assert_eq!(
        drawing.drawn(drawn(4, 200.0, 201.0, 360.0)),
        Record::Drawn {
            id: 4,
            decode_ms: 1.0,
            draw_ms: 159.0,
            gap_ms: Some(250.0)
        }
    );
}

#[test]
fn many_drawn_units_measure_the_gap_from_the_one_before() {
    let mut drawing: Ledger = Ledger::default();
    drawing.drawn(drawn(3, 0.0, 0.0, 10.0));
    drawing.drawn(drawn(4, 0.0, 0.0, 30.0));
    assert_eq!(
        drawing.drawn(drawn(5, 0.0, 0.0, 70.0)),
        Record::Drawn {
            id: 5,
            decode_ms: 0.0,
            draw_ms: 70.0,
            gap_ms: Some(40.0)
        }
    );
}

#[test]
fn a_unit_record_serializes_tagged() {
    assert_eq!(
        serde_json::to_string(&unit(4, false, 2.5, 60.0)).ok(),
        Some(String::from(
            r#"{"t":"unit","id":4,"key":false,"bytes":104,"capture_send_ms":2.5,"send_ack_ms":60.0}"#
        ))
    );
}

#[test]
fn a_first_drawn_record_serializes_a_null_gap() {
    assert_eq!(
        serde_json::to_string(&Ledger::default().drawn(drawn(3, 100.0, 104.5, 110.0))).ok(),
        Some(String::from(
            r#"{"t":"drawn","id":3,"decode_ms":4.5,"draw_ms":5.5,"gap_ms":null}"#
        ))
    );
}
