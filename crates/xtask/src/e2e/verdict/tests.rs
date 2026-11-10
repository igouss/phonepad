use super::{
    CURSOR_TAG, Decoded, FRAME_HEADER, FRAME_TAG, POINTER_MESSAGE, Tally, distinct, frame_hashes,
    shutdown_check, status_check, tally, viewer_checks,
};
use crate::report::Check;
use crate::testbed::Shutdown;
use std::time::Duration;

const IDR: [u8; 5] = [0, 0, 0, 1, 0x65];
const SLICE: [u8; 5] = [0, 0, 0, 1, 0x41];
const LIMIT: Duration = Duration::from_secs(5);

fn message(unit: &[u8]) -> Vec<u8> {
    [FRAME_TAG]
        .iter()
        .chain(&[0u8; FRAME_HEADER - 1])
        .chain(unit)
        .copied()
        .collect()
}

fn cursor() -> Vec<u8> {
    [CURSOR_TAG]
        .iter()
        .chain(&[0u8; POINTER_MESSAGE - 1])
        .copied()
        .collect()
}

fn decoded(frames: usize, distinct: usize, errors: &str) -> Decoded {
    Decoded {
        frames,
        distinct,
        errors: errors.to_owned(),
    }
}

fn units(count: usize) -> Tally {
    Tally {
        units: count,
        first_unit_key: Some(true),
        ..Tally::default()
    }
}

fn verdicts(tally: &Tally, decoded: &Decoded) -> [bool; 4] {
    viewer_checks("v", tally, decoded, LIMIT).map(|check: Check| check.ok)
}

#[test]
fn no_messages_tally_nothing() {
    assert_eq!(tally(&[], 0), Tally::default());
}

#[test]
fn one_keyframe_message_is_one_key_unit() {
    assert_eq!(
        tally(&[message(&IDR)], 0),
        Tally {
            units: 1,
            keys: 1,
            first_unit_key: Some(true),
            stream: IDR.to_vec(),
            ..Tally::default()
        }
    );
}

#[test]
fn units_concatenate_into_one_stream_and_count_keys() {
    let counted: Tally = tally(&[message(&IDR), message(&SLICE), message(&IDR)], 0);
    assert_eq!(
        (counted.units, counted.keys, counted.stream),
        (3, 2, [IDR, SLICE, IDR].concat())
    );
}

#[test]
fn a_delta_first_is_remembered_after_a_later_key() {
    assert_eq!(
        tally(&[message(&SLICE), message(&IDR)], 0).first_unit_key,
        Some(false)
    );
}

#[test]
fn a_bare_header_without_a_unit_is_malformed() {
    assert_eq!(tally(&[message(&[])], 0).malformed, 1);
}

#[test]
fn a_message_shorter_than_the_header_is_malformed() {
    assert_eq!(tally(&[vec![0u8; FRAME_HEADER - 1]], 0).malformed, 1);
}

#[test]
fn a_cursor_message_is_counted_and_is_not_a_unit() {
    assert_eq!(
        tally(&[cursor(), message(&IDR), cursor()], 0),
        Tally {
            units: 1,
            keys: 1,
            cursors: 2,
            first_unit_key: Some(true),
            stream: IDR.to_vec(),
            ..Tally::default()
        }
    );
}

#[test]
fn a_cursor_message_with_trailing_bytes_is_malformed() {
    let mut long: Vec<u8> = cursor();
    long.push(0);
    assert_eq!(tally(&[long], 0).malformed, 1);
}

#[test]
fn an_unknown_tag_is_malformed() {
    let mut unknown: Vec<u8> = message(&IDR);
    unknown[0] = 2;
    assert_eq!(tally(&[unknown], 0).malformed, 1);
}

#[test]
fn an_empty_message_is_malformed() {
    assert_eq!(tally(&[Vec::new()], 0).malformed, 1);
}

#[test]
fn text_messages_count_as_malformed() {
    assert_eq!(tally(&[message(&IDR)], 2).malformed, 2);
}

#[test]
fn framemd5_without_frames_has_no_hashes() {
    assert_eq!(
        frame_hashes("#format: frame checksums\n#version: 2\n\n"),
        Vec::<&str>::new()
    );
}

#[test]
fn framemd5_frame_lines_yield_their_last_field() {
    assert_eq!(
        frame_hashes(
            "#tb 0: 1/25\n0,          0,          0,        1,  2083968, aa11\n0,          1,          1,        1,  2083968, bb22\n"
        ),
        vec!["aa11", "bb22"]
    );
}

#[test]
fn no_hashes_are_zero_distinct() {
    assert_eq!(distinct(&[]), 0);
}

#[test]
fn one_hash_is_one_distinct() {
    assert_eq!(distinct(&["a"]), 1);
}

#[test]
fn repeated_hashes_count_once() {
    assert_eq!(distinct(&["a", "b", "a", "b", "c"]), 3);
}

#[test]
fn a_decoded_viewer_passes_every_check() {
    assert_eq!(verdicts(&units(2), &decoded(2, 1, "")), [true; 4]);
}

#[test]
fn a_viewer_without_units_fails_units_and_decoding() {
    assert_eq!(
        verdicts(&Tally::default(), &decoded(0, 0, "")),
        [false, false, false, false]
    );
}

#[test]
fn a_malformed_message_fails_the_units_check() {
    let tally: Tally = Tally {
        malformed: 1,
        ..units(1)
    };
    assert_eq!(
        verdicts(&tally, &decoded(1, 1, "")),
        [false, true, true, true]
    );
}

#[test]
fn a_delta_first_fails_the_key_check() {
    let tally: Tally = Tally {
        first_unit_key: Some(false),
        ..units(1)
    };
    assert_eq!(
        verdicts(&tally, &decoded(1, 1, "")),
        [true, false, true, true]
    );
}

#[test]
fn decoder_errors_fail_decoding() {
    assert_eq!(
        verdicts(&units(1), &decoded(1, 1, "corrupt")),
        [true, true, false, true]
    );
}

#[test]
fn fewer_frames_than_units_fail_decoding() {
    assert_eq!(
        verdicts(&units(2), &decoded(1, 1, "")),
        [true, true, false, true]
    );
}

#[test]
fn units_are_reported_per_second() {
    assert!(
        viewer_checks("v", &units(10), &decoded(10, 1, ""), Duration::from_secs(4))[0]
            .detail
            .starts_with("10 units (2.5/s")
    );
}

#[test]
fn checks_carry_the_viewer_label() {
    assert_eq!(
        viewer_checks("3-viewers/2", &units(1), &decoded(1, 1, ""), LIMIT)[3].gate,
        "3-viewers/2 distinct"
    );
}

#[test]
fn forbidden_passes_the_status_check() {
    assert!(status_check("g", Ok(403)).ok);
}

#[test]
fn an_upgrade_fails_the_status_check() {
    assert_eq!(
        status_check("g", Ok(101)),
        Check {
            gate: String::from("g"),
            ok: false,
            detail: String::from("HTTP 101, expected 403")
        }
    );
}

#[test]
fn a_transport_error_fails_the_status_check() {
    assert!(!status_check("g", Err(String::from("refused"))).ok);
}

fn exited(millis: u64, code: Option<i32>) -> Shutdown {
    Shutdown::Exited {
        after: Duration::from_millis(millis),
        code,
    }
}

#[test]
fn a_clean_exit_within_the_limit_passes() {
    assert!(shutdown_check(&exited(60, Some(0)), LIMIT).ok);
}

#[test]
fn an_exit_at_the_limit_passes() {
    assert!(shutdown_check(&exited(5000, Some(0)), LIMIT).ok);
}

#[test]
fn an_exit_past_the_limit_fails() {
    assert!(!shutdown_check(&exited(5001, Some(0)), LIMIT).ok);
}

#[test]
fn a_failing_exit_code_fails() {
    assert!(!shutdown_check(&exited(60, Some(1)), LIMIT).ok);
}

#[test]
fn death_by_signal_fails() {
    assert!(!shutdown_check(&exited(60, None), LIMIT).ok);
}

#[test]
fn a_hang_fails() {
    assert!(
        !shutdown_check(
            &Shutdown::Hung {
                waited: Duration::from_secs(20)
            },
            LIMIT
        )
        .ok
    );
}

#[test]
fn an_exit_before_sigterm_fails() {
    assert!(!shutdown_check(&Shutdown::Early { code: Some(0) }, LIMIT).ok);
}
