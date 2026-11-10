use super::*;

fn unit(id: u32, bytes: u32, at_us: u64) -> Sent {
    Sent { id, bytes, at_us }
}

fn filled(units: &[Sent]) -> Pipe {
    units.iter().copied().fold(Pipe::default(), Pipe::sent)
}

fn many(count: u32, bytes: u32) -> Pipe {
    (1..=count).fold(Pipe::default(), |pipe: Pipe, id: u32| {
        pipe.sent(unit(id, bytes, u64::from(id)))
    })
}

fn measured(rate: u64, min_rtt_us: u64) -> Pipe {
    Pipe {
        estimate: Some(Estimate {
            min_rtt_us: Filtered {
                value: min_rtt_us,
                at_us: 0,
            },
            rate: Filtered {
                value: rate,
                at_us: 0,
            },
        }),
        ..Pipe::default()
    }
}

fn estimate(pipe: &Pipe) -> Option<(u64, u64)> {
    pipe.estimate
        .map(|estimate: Estimate| (estimate.min_rtt_us.value, estimate.rate.value))
}

const BIG: u32 = 1 << 20;

#[test]
fn a_new_pipe_is_empty_and_carries_nothing() {
    let pipe: Pipe = Pipe::default();
    assert_eq!(
        (
            pipe.is_empty(),
            pipe.carries(1),
            pipe.ends_with(1),
            pipe.in_flight()
        ),
        (true, false, false, 0)
    );
}

#[test]
fn one_sent_unit_is_carried_and_counted() {
    let pipe: Pipe = filled(&[unit(7, 300, 10)]);
    assert_eq!(
        (
            pipe.is_empty(),
            pipe.carries(7),
            pipe.ends_with(7),
            pipe.in_flight()
        ),
        (false, true, true, 300)
    );
}

#[test]
fn many_sent_units_are_counted_and_only_the_newest_ends_the_flight() {
    let pipe: Pipe = filled(&[unit(1, 300, 10), unit(2, 500, 20), unit(3, 700, 30)]);
    assert_eq!(
        (
            pipe.carries(1),
            pipe.carries(4),
            pipe.ends_with(1),
            pipe.ends_with(3),
            pipe.in_flight()
        ),
        (true, false, false, true, 1500)
    );
}

#[test]
fn an_unmeasured_link_gets_the_floor() {
    assert_eq!(Pipe::default().window(), FLOOR_BYTES);
}

#[test]
fn a_measured_link_gets_its_rate_over_min_rtt_plus_the_queue_cap() {
    assert_eq!(measured(2_000_000, 40_000).window(), 200_000);
}

#[test]
fn a_slow_link_still_gets_the_floor() {
    assert_eq!(measured(100_000, 40_000).window(), FLOOR_BYTES);
}

#[test]
fn a_fast_link_is_capped_at_the_ceiling() {
    assert_eq!(measured(1_000_000_000, 40_000).window(), 4_194_304);
}

#[test]
fn an_empty_pipe_admits_a_unit_larger_than_any_window() {
    assert!(Pipe::default().admits(u32::MAX));
}

#[test]
fn a_unit_that_fills_the_window_exactly_is_admitted() {
    let pipe: Pipe = filled(&[unit(1, 1000, 0)]);
    assert!(pipe.admits(u32::try_from(FLOOR_BYTES).unwrap() - 1000));
}

#[test]
fn a_unit_one_byte_over_the_window_is_refused() {
    let pipe: Pipe = filled(&[unit(1, 1000, 0)]);
    assert!(!pipe.admits(u32::try_from(FLOOR_BYTES).unwrap() - 999));
}

#[test]
fn the_flight_below_its_unit_limit_admits_one_more() {
    assert!(many(u32::try_from(MAX_UNITS).unwrap() - 1, 1).admits(1));
}

#[test]
fn the_flight_at_its_unit_limit_admits_nothing_however_small() {
    assert!(!many(u32::try_from(MAX_UNITS).unwrap(), 1).admits(0));
}

#[test]
fn occupancy_is_empty_room_or_full() {
    assert_eq!(
        [
            Pipe::default().occupancy(),
            filled(&[unit(1, 1000, 0)]).occupancy(),
            filled(&[unit(1, BIG, 0)]).occupancy()
        ],
        [Occupancy::Empty, Occupancy::Room, Occupancy::Full]
    );
}

#[test]
fn an_ack_for_an_unknown_unit_changes_nothing() {
    let pipe: Pipe = filled(&[unit(1, 300, 10)]);
    assert_eq!(pipe.clone().acked(9, 500), pipe);
}

#[test]
fn an_ack_lands_its_unit_and_every_unit_sent_before_it() {
    let pipe: Pipe = filled(&[unit(1, 300, 10), unit(2, 500, 20), unit(3, 700, 30)]).acked(2, 100);
    assert_eq!(
        (
            pipe.carries(1),
            pipe.carries(2),
            pipe.carries(3),
            pipe.in_flight()
        ),
        (false, false, true, 700)
    );
}

#[test]
fn the_first_ack_measures_the_round_trip_and_the_rate_from_the_send() {
    let pipe: Pipe = filled(&[unit(1, 5000, 1_000_000)]).acked(1, 1_050_000);
    assert_eq!(estimate(&pipe), Some((50_000, 100_000)));
}

#[test]
fn units_sent_behind_a_unit_in_flight_share_its_start() {
    let pipe: Pipe = filled(&[unit(1, 5000, 1_000_000)])
        .acked(1, 1_050_000)
        .sent(unit(2, 5000, 1_060_000))
        .sent(unit(3, 5000, 1_070_000))
        .acked(2, 1_110_000)
        .acked(3, 1_120_000);
    assert_eq!(estimate(&pipe), Some((50_000, 166_666)));
}

#[test]
fn a_unit_sent_after_a_delivery_into_a_busy_pipe_is_timed_from_that_delivery() {
    let pipe: Pipe = filled(&[unit(1, 1000, 0), unit(2, 1000, 10_000)])
        .acked(1, 50_000)
        .sent(unit(3, 1000, 60_000))
        .acked(2, 70_000)
        .acked(3, 110_000);
    assert_eq!(estimate(&pipe), Some((50_000, 33_333)));
}

#[test]
fn a_unit_sent_into_an_empty_pipe_is_timed_from_its_own_send() {
    let pipe: Pipe = filled(&[unit(1, 5000, 1_000_000)])
        .acked(1, 1_050_000)
        .sent(unit(2, 5000, 3_000_000))
        .acked(2, 3_025_000);
    assert_eq!(estimate(&pipe), Some((25_000, 200_000)));
}

#[test]
fn a_cumulative_ack_counts_every_landed_byte_in_the_rate() {
    let pipe: Pipe =
        filled(&[unit(1, 4000, 1_000_000), unit(2, 1000, 1_000_000)]).acked(2, 1_050_000);
    assert_eq!(estimate(&pipe), Some((50_000, 100_000)));
}

#[test]
fn a_longer_round_trip_and_a_lower_rate_keep_the_held_estimate() {
    let pipe: Pipe = filled(&[unit(1, 5000, 1_000_000)])
        .acked(1, 1_050_000)
        .sent(unit(2, 1000, 1_100_000))
        .acked(2, 1_200_000);
    assert_eq!(estimate(&pipe), Some((50_000, 100_000)));
}

#[test]
fn a_shorter_round_trip_and_a_higher_rate_replace_the_held_estimate() {
    let pipe: Pipe = filled(&[unit(1, 5000, 1_000_000)])
        .acked(1, 1_050_000)
        .sent(unit(2, 9000, 1_100_000))
        .acked(2, 1_130_000);
    assert_eq!(estimate(&pipe), Some((30_000, 300_000)));
}

#[test]
fn an_estimate_held_for_exactly_its_window_is_kept() {
    let pipe: Pipe = filled(&[unit(1, 5000, 0)])
        .acked(1, 50_000)
        .sent(unit(2, 1000, RATE_WINDOW_US))
        .acked(2, RATE_WINDOW_US + 50_000);
    assert_eq!(estimate(&pipe), Some((50_000, 100_000)));
}

#[test]
fn a_rate_held_longer_than_its_window_gives_way_to_the_latest_sample() {
    let pipe: Pipe = filled(&[unit(1, 5000, 0)])
        .acked(1, 50_000)
        .sent(unit(2, 1000, RATE_WINDOW_US))
        .acked(2, RATE_WINDOW_US + 50_001);
    assert_eq!(estimate(&pipe), Some((50_000, 19_999)));
}

#[test]
fn a_min_rtt_held_longer_than_its_window_gives_way_to_the_latest_sample() {
    let pipe: Pipe = filled(&[unit(1, 5000, 0)])
        .acked(1, 50_000)
        .sent(unit(2, 5000, MIN_RTT_WINDOW_US))
        .acked(2, MIN_RTT_WINDOW_US + 60_001);
    assert_eq!(estimate(&pipe), Some((60_001, 83_331)));
}

#[test]
fn an_ack_at_the_send_instant_counts_as_one_microsecond() {
    let pipe: Pipe = filled(&[unit(1, 5000, 1_000)]).acked(1, 1_000);
    assert_eq!(estimate(&pipe), Some((0, 5_000_000_000)));
}

#[test]
fn the_measured_estimate_sizes_the_window() {
    let pipe: Pipe = filled(&[unit(1, 40_000, 1_000_000)]).acked(1, 1_040_000);
    assert_eq!(pipe.window(), 1_000_000 * 100_000 / 1_000_000);
}
