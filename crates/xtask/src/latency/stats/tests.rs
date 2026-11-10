use super::*;
use crate::latency::link;

fn unit(bytes: u64, capture_send: f64, send_ack: f64) -> Sample {
    Sample::Unit {
        bytes,
        capture_send,
        send_ack,
    }
}

fn drawn(decode: f64, draw: f64, gap: Option<f64>) -> Sample {
    Sample::Drawn { decode, draw, gap }
}

fn spread_of(p50: f64, p95: f64, max: f64) -> Option<Spread> {
    Some(Spread { p50, p95, max })
}

fn acked(p50: f64) -> Summary {
    Summary {
        send_ack: spread_of(p50, p50, p50),
        ..summarize(&[], Duration::from_secs(1))
    }
}

#[test]
fn a_unit_line_is_a_unit_sample() {
    assert_eq!(
        sample(
            r#"timing {"t":"unit","id":4,"key":false,"bytes":104,"capture_send_ms":2.5,"send_ack_ms":60.0}"#
        ),
        Some(unit(104, 2.5, 60.0))
    );
}

#[test]
fn a_drawn_line_is_a_drawn_sample() {
    assert_eq!(
        sample(r#"timing {"t":"drawn","id":3,"decode_ms":4.5,"draw_ms":5.5,"gap_ms":40.0}"#),
        Some(drawn(4.5, 5.5, Some(40.0)))
    );
}

#[test]
fn a_first_drawn_line_has_no_gap() {
    assert_eq!(
        sample(r#"timing {"t":"drawn","id":3,"decode_ms":4.5,"draw_ms":5.5,"gap_ms":null}"#),
        Some(drawn(4.5, 5.5, None))
    );
}

#[test]
fn a_line_without_the_prefix_is_no_sample() {
    assert_eq!(sample("phonepad listening on http://127.0.0.1:8771"), None);
}

#[test]
fn a_record_of_another_type_is_no_sample() {
    assert_eq!(sample(r#"timing {"t":"stall"}"#), None);
}

#[test]
fn a_unit_missing_a_field_is_no_sample() {
    assert_eq!(
        sample(r#"timing {"t":"unit","bytes":104,"capture_send_ms":2.5}"#),
        None
    );
}

#[test]
fn a_drawn_record_missing_a_field_is_no_sample() {
    assert_eq!(sample(r#"timing {"t":"drawn","decode_ms":4.5}"#), None);
}

#[test]
fn malformed_json_is_no_sample() {
    assert_eq!(sample("timing {"), None);
}

#[test]
fn no_values_have_no_spread() {
    assert_eq!(spread(Vec::new()), None);
}

#[test]
fn one_value_is_every_percentile() {
    assert_eq!(spread(vec![7.0]), spread_of(7.0, 7.0, 7.0));
}

#[test]
fn many_values_take_the_nearest_rank() {
    assert_eq!(
        spread((1..=20).rev().map(f64::from).collect()),
        spread_of(10.0, 19.0, 20.0)
    );
}

#[test]
fn two_values_split_at_the_lower() {
    assert_eq!(spread(vec![3.0, 1.0]), spread_of(1.0, 3.0, 3.0));
}

#[test]
fn no_samples_summarize_to_zero_rates_and_no_spreads() {
    assert_eq!(
        summarize(&[], Duration::from_secs(2)),
        Summary {
            units_per_s: 0.0,
            mbit_per_s: 0.0,
            drawn_per_s: 0.0,
            stalls: 0,
            longest_gap: None,
            capture_send: None,
            send_ack: None,
            capture_ack: None,
            decode: None,
            draw: None,
        }
    );
}

#[test]
fn units_and_draws_summarize_per_second_and_per_stage() {
    assert_eq!(
        summarize(
            &[
                unit(250_000, 2.0, 50.0),
                drawn(3.0, 8.0, None),
                unit(250_000, 4.0, 70.0),
                drawn(5.0, 6.0, Some(250.0)),
                drawn(1.0, 2.0, Some(251.0)),
                drawn(1.0, 2.0, Some(40.0)),
            ],
            Duration::from_secs(2)
        ),
        Summary {
            units_per_s: 1.0,
            mbit_per_s: 2.0,
            drawn_per_s: 2.0,
            stalls: 1,
            longest_gap: Some(251.0),
            capture_send: spread_of(2.0, 4.0, 4.0),
            send_ack: spread_of(50.0, 70.0, 70.0),
            capture_ack: spread_of(52.0, 74.0, 74.0),
            decode: spread_of(1.0, 5.0, 5.0),
            draw: spread_of(2.0, 8.0, 8.0),
        }
    );
}

#[test]
fn a_row_names_the_link_and_rounds_each_stage() {
    assert_eq!(
        row(
            &link::named("bad-lte").unwrap(),
            &summarize(
                &[unit(1_000, 2.4, 160.6), drawn(3.0, 8.0, None)],
                Duration::from_secs(1)
            )
        ),
        "| bad-lte | 150 ms ±50 ms, 3 Mbit/s, 1% loss | 1.0 | 0.01 | 1.0 | 0 (-) | 2 / 2 / 2 | 161 / 161 / 161 | 163 / 163 / 163 | 3 / 3 / 3 | 8 / 8 / 8 |"
    );
}

#[test]
fn an_uncapped_row_says_so_and_shows_no_stage_it_lacks() {
    assert_eq!(
        row(
            &link::named("lan").unwrap(),
            &summarize(&[], Duration::from_secs(1))
        ),
        "| lan | 0 ms ±0 ms, uncapped, 0% loss | 0.0 | 0.00 | 0.0 | 0 (-) | - | - | - | - | - |"
    );
}

#[test]
fn the_added_rtt_shows_in_the_median() {
    assert!(added_rtt_check(100.0, &acked(20.0), &acked(120.0)).ok);
}

#[test]
fn the_added_rtt_may_miss_by_the_tolerance() {
    assert_eq!(
        [
            added_rtt_check(100.0, &acked(20.0), &acked(135.0)).ok,
            added_rtt_check(100.0, &acked(20.0), &acked(105.0)).ok
        ],
        [true, true]
    );
}

#[test]
fn the_added_rtt_beyond_the_tolerance_fails() {
    assert_eq!(
        [
            added_rtt_check(100.0, &acked(20.0), &acked(135.5)).ok,
            added_rtt_check(100.0, &acked(20.0), &acked(104.5)).ok
        ],
        [false, false]
    );
}

#[test]
fn the_added_rtt_check_names_its_delay_and_the_medians() {
    assert_eq!(
        added_rtt_check(200.0, &acked(20.0), &acked(221.0)),
        Check {
            gate: String::from("added-rtt-200"),
            ok: true,
            detail: String::from(
                "send→ack p50 221.0 ms vs 20.0 ms at no added RTT: +201.0 ms, expected +200 ± 15"
            ),
        }
    );
}

#[test]
fn the_added_rtt_check_fails_without_acked_units() {
    assert!(!added_rtt_check(100.0, &acked(20.0), &summarize(&[], Duration::from_secs(1))).ok);
}

#[test]
fn a_rate_at_the_ceiling_passes_the_cap() {
    assert!(cap_check("c", 2.1, None, 2.1).ok);
}

#[test]
fn a_rate_above_the_ceiling_fails_the_cap() {
    assert!(!cap_check("c", 2.11, None, 2.1).ok);
}

#[test]
fn a_rate_below_the_floor_fails_the_cap() {
    assert_eq!(
        [
            cap_check("c", 1.89, Some(1.9), 2.1).ok,
            cap_check("c", 1.9, Some(1.9), 2.1).ok
        ],
        [false, true]
    );
}

#[test]
fn the_cap_check_states_its_bounds() {
    assert_eq!(
        cap_check("bulk-cap", 1.95, Some(1.9), 2.1).detail,
        "1.950 Mbit/s, expected at least 1.9 and at most 2.1 Mbit/s"
    );
}
