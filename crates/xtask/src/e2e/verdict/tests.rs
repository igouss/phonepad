use super::{
    CURSOR_TAG, Decoded, FRAME_HEADER, FRAME_TAG, HEADER, Shutdown, Tally, distinct, executable,
    frame_hashes, instance, listening, shutdown_check, spawned, status_check, tally, viewer_checks,
};
use crate::report::Check;
use std::ffi::OsStr;
use std::path::PathBuf;
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
        .chain(&[0u8; HEADER - 1])
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
    assert_eq!(tally(&[vec![0u8; HEADER - 1]], 0).malformed, 1);
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
fn the_listen_line_names_the_port() {
    assert!(listening(
        "phonepad listening on http://127.0.0.1:8769",
        8769
    ));
}

#[test]
fn another_port_is_not_listening() {
    assert!(!listening(
        "phonepad listening on http://127.0.0.1:8765",
        8769
    ));
}

fn live(signatures: &[&str]) -> Vec<String> {
    signatures.iter().map(|s: &&str| (*s).to_owned()).collect()
}

#[test]
fn no_live_instance_is_an_error() {
    assert!(instance(Some("a"), &[]).is_err());
}

#[test]
fn a_stale_inherited_signature_yields_the_one_live_instance() {
    assert_eq!(
        instance(Some("stale"), &live(&["live"])),
        Ok(String::from("live"))
    );
}

#[test]
fn no_inherited_signature_yields_the_one_live_instance() {
    assert_eq!(instance(None, &live(&["live"])), Ok(String::from("live")));
}

#[test]
fn a_live_inherited_signature_wins_among_many() {
    assert_eq!(
        instance(Some("b"), &live(&["a", "b", "c"])),
        Ok(String::from("b"))
    );
}

#[test]
fn many_live_instances_without_a_live_inherited_one_are_ambiguous() {
    assert!(instance(Some("stale"), &live(&["a", "b"])).is_err());
}

fn argv() -> [&'static OsStr; 3] {
    [
        OsStr::new("/b/phonepad"),
        OsStr::new("8769"),
        OsStr::new("test@example.invalid"),
    ]
}

#[test]
fn the_spawned_command_line_matches() {
    assert!(spawned(
        b"/b/phonepad\x008769\x00test@example.invalid\x00",
        &argv()
    ));
}

#[test]
fn an_empty_command_line_is_not_ours() {
    assert!(!spawned(b"", &argv()));
}

#[test]
fn another_port_is_not_ours() {
    assert!(!spawned(
        b"/b/phonepad\x008765\x00test@example.invalid\x00",
        &argv()
    ));
}

#[test]
fn an_extra_argument_is_not_ours() {
    assert!(!spawned(
        b"/b/phonepad\x008769\x00test@example.invalid\x00x\x00",
        &argv()
    ));
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

const ARTIFACT: &str = r#"{"reason":"compiler-artifact","target":{"name":"phonepad"},"executable":"/t/debug/phonepad"}"#;

#[test]
fn no_cargo_messages_name_no_executable() {
    assert_eq!(executable("", "phonepad"), None);
}

#[test]
fn the_artifact_names_the_executable() {
    assert_eq!(
        executable(ARTIFACT, "phonepad"),
        Some(PathBuf::from("/t/debug/phonepad"))
    );
}

#[test]
fn other_targets_and_reasons_are_skipped() {
    let messages: String = [
        r#"{"reason":"compiler-artifact","target":{"name":"phonepad-core"},"executable":"/t/x"}"#,
        r#"{"reason":"build-finished","target":{"name":"phonepad"},"executable":"/t/y"}"#,
        "not json",
        ARTIFACT,
        r#"{"reason":"compiler-artifact","target":{"name":"phonepad"},"executable":null}"#,
    ]
    .join("\n");
    assert_eq!(
        executable(&messages, "phonepad"),
        Some(PathBuf::from("/t/debug/phonepad"))
    );
}

#[test]
fn a_later_artifact_of_another_target_is_skipped() {
    let messages: String = [
        ARTIFACT,
        r#"{"reason":"compiler-artifact","target":{"name":"xtask"},"executable":"/t/debug/xtask"}"#,
    ]
    .join("\n");
    assert_eq!(
        executable(&messages, "phonepad"),
        Some(PathBuf::from("/t/debug/phonepad"))
    );
}
