use super::*;

fn spread(p50: f64, p95: f64, max: f64) -> Option<Spread> {
    Some(Spread { p50, p95, max })
}

fn measured(redraw_p95: f64, reconciled: usize, backward_max: Option<f64>) -> Phase {
    Phase {
        redraw: spread(1.0, redraw_p95, redraw_p95),
        queued_redraw: spread(20.0, 25.0, 30.0),
        reconciled,
        backward_max,
        jump_max: backward_max,
    }
}

fn verdicts(phase: &Phase) -> [bool; 2] {
    checks("lte", phase).map(|check: Check| check.ok)
}

#[test]
fn a_jump_along_the_heading_is_not_backwards() {
    assert_eq!(backward([3.0, 4.0, 6.0, 8.0]), 0.0);
}

#[test]
fn a_jump_against_the_heading_is_backwards_by_its_length_along_it() {
    assert_eq!(backward([-3.0, -4.0, 6.0, 8.0]), 5.0);
}

#[test]
fn a_jump_across_the_heading_is_not_backwards() {
    assert_eq!(backward([0.0, 7.0, 5.0, 0.0]), 0.0);
}

#[test]
fn a_jump_without_a_heading_counts_whole() {
    assert_eq!(backward([3.0, -4.0, 0.0, 0.0]), 5.0);
}

#[test]
fn no_records_measure_nothing() {
    assert_eq!(
        phase(&Taken::default()),
        Phase {
            redraw: None,
            queued_redraw: None,
            reconciled: 0,
            backward_max: None,
            jump_max: None
        }
    );
}

#[test]
fn one_draw_times_redraw_from_the_handler_and_from_the_event() {
    let taken: Taken = Taken {
        draws: vec![[100.0, 110.0, 112.5]],
        jumps: Vec::new(),
    };
    assert_eq!(
        (phase(&taken).redraw, phase(&taken).queued_redraw),
        (spread(2.5, 2.5, 2.5), spread(12.5, 12.5, 12.5))
    );
}

#[test]
fn many_jumps_keep_the_largest_backward_and_the_largest_overall() {
    let taken: Taken = Taken {
        draws: Vec::new(),
        jumps: vec![
            [-1.0, 0.0, 1.0, 0.0],
            [9.0, 0.0, 1.0, 0.0],
            [-2.0, 0.0, 1.0, 0.0],
        ],
    };
    assert_eq!(
        (
            phase(&taken).reconciled,
            phase(&taken).backward_max,
            phase(&taken).jump_max
        ),
        (3, Some(2.0), Some(9.0))
    );
}

#[test]
fn the_probe_record_parses() {
    assert_eq!(
        parse(r#"{"draws":[[1,2,3]],"jumps":[[1,2,3,4]]}"#).ok(),
        Some(Taken {
            draws: vec![[1.0, 2.0, 3.0]],
            jumps: vec![[1.0, 2.0, 3.0, 4.0]]
        })
    );
}

#[test]
fn a_probe_row_of_the_wrong_length_is_an_error() {
    assert!(parse(r#"{"draws":[[1,2]],"jumps":[]}"#).is_err());
}

#[test]
fn a_probe_record_without_jumps_is_an_error() {
    assert!(parse(r#"{"draws":[]}"#).is_err());
}

#[test]
fn the_finger_starts_right_of_the_centre() {
    assert_eq!(finger((100.0, 200.0), 0), (100.0 + RADIUS_CSS_PX, 200.0));
}

#[test]
fn a_quarter_turn_puts_the_finger_below_the_centre() {
    let (x, y): (f64, f64) = finger((100.0, 200.0), STEPS_PER_TURN / 4);
    assert_eq!(((x - 100.0).abs() < 1e-9, y), (true, 200.0 + RADIUS_CSS_PX));
}

#[test]
fn a_whole_turn_brings_the_finger_back() {
    assert_eq!(
        finger((100.0, 200.0), STEPS_PER_TURN + 1),
        finger((100.0, 200.0), 1)
    );
}

#[test]
fn a_redraw_at_the_limit_with_a_small_backward_jump_passes() {
    assert_eq!(
        verdicts(&measured(REDRAW_P95_MS, 1, Some(BACKWARD_MAX_PX))),
        [true, true]
    );
}

#[test]
fn a_redraw_over_the_limit_fails() {
    assert_eq!(
        verdicts(&measured(REDRAW_P95_MS + 0.1, 1, Some(0.0))),
        [false, true]
    );
}

#[test]
fn a_backward_jump_over_the_limit_fails() {
    assert_eq!(
        verdicts(&measured(1.0, 1, Some(BACKWARD_MAX_PX + 0.1))),
        [true, false]
    );
}

#[test]
fn no_draws_and_no_pointer_messages_fail() {
    assert_eq!(
        verdicts(&Phase {
            redraw: None,
            queued_redraw: None,
            reconciled: 0,
            backward_max: None,
            jump_max: None
        }),
        [false, false]
    );
}
