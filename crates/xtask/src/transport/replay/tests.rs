use super::*;

fn arrival(seq: u32) -> Arrival {
    Arrival {
        seq,
        id: seq + 100,
        sent_us: 0,
        bytes: 1000,
    }
}

fn seqs(released: &[Arrival]) -> Vec<u32> {
    released
        .iter()
        .map(|arrival: &Arrival| arrival.seq)
        .collect()
}

#[test]
fn the_next_unit_in_order_is_released_at_once() {
    let mut reorder: Reorder = Reorder::default();
    assert_eq!(seqs(&reorder.arrive(arrival(0))), vec![0]);
}

#[test]
fn a_unit_ahead_of_a_missing_one_is_held() {
    let mut reorder: Reorder = Reorder::default();
    assert_eq!(seqs(&reorder.arrive(arrival(1))), Vec::<u32>::new());
}

#[test]
fn the_missing_unit_releases_every_unit_held_behind_it() {
    let mut reorder: Reorder = Reorder::default();
    reorder.arrive(arrival(2));
    reorder.arrive(arrival(1));
    assert_eq!(seqs(&reorder.arrive(arrival(0))), vec![0, 1, 2]);
}

fn released(at_ms: u64) -> Released {
    Released {
        at_us: at_ms * 1000,
        sent_us: at_ms * 1000 - 100_000,
        bytes: 1000,
    }
}

fn log(at_ms: &[u64]) -> Log {
    Log {
        released: at_ms.iter().map(|at: &u64| released(*at)).collect(),
        ..Log::default()
    }
}

#[test]
fn a_gap_over_250_ms_is_a_stall_but_not_over_500() {
    let outcome: Outcome = outcome(&log(&[1000, 1300]), 1_000_000, 2_000_000);
    assert_eq!((outcome.stalls_250, outcome.stalls_500), (1, 0));
}

#[test]
fn a_gap_over_500_ms_counts_in_both() {
    let outcome: Outcome = outcome(&log(&[1000, 1600]), 1_000_000, 2_000_000);
    assert_eq!((outcome.stalls_250, outcome.stalls_500), (1, 1));
}

#[test]
fn a_gap_of_exactly_250_ms_is_no_stall() {
    let outcome: Outcome = outcome(&log(&[1000, 1250]), 1_000_000, 2_000_000);
    assert_eq!(outcome.stalls_250, 0);
}

#[test]
fn units_outside_the_window_are_not_counted() {
    let outcome: Outcome = outcome(&log(&[500, 1000, 1500, 2500]), 1_000_000, 2_000_000);
    assert_eq!(outcome.units_per_s, 2.0);
}

#[test]
fn send_to_draw_is_release_minus_send() {
    let outcome: Outcome = outcome(&log(&[1000]), 1_000_000, 2_000_000);
    assert_eq!(outcome.send_draw.map(|s: Spread| s.p50), Some(100.0));
}

#[test]
fn an_unanswered_ping_is_counted() {
    let pinged: Log = Log {
        pings: vec![(1_100_000, Some(1_250_000)), (1_200_000, None)],
        ..Log::default()
    };
    let outcome: Outcome = outcome(&pinged, 1_000_000, 2_000_000);
    assert_eq!(
        (
            outcome.ping.map(|s: Spread| s.p50),
            outcome.pings_unanswered
        ),
        (Some(150.0), 1)
    );
}

#[test]
fn a_unit_message_parses_back_to_its_arrival() {
    assert_eq!(
        parse_unit(&unit_message(3, 9, 77, 1500)),
        Some(Arrival {
            seq: 3,
            id: 9,
            sent_us: 77,
            bytes: 1500
        })
    );
}

#[test]
fn deltas_stay_within_their_size_range() {
    let mut sizes: Sizes = Sizes {
        state: 7,
        deltas: (200, 600),
    };
    let drawn: Vec<u32> = (0..1000).map(|_| sizes.next(UnitKind::Delta)).collect();
    assert!(drawn.iter().all(|bytes: &u32| (200..600).contains(bytes)));
}
