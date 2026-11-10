use super::{
    Rates, Shown, Stream, changed, check, expected, per_second, row, sample_at, stream, target,
};
use crate::viewer::{CURSOR_TAG, FRAME_TAG};
use std::collections::BTreeSet;
use std::time::Duration;

fn frame(id: u32, (x, y): (i32, i32), unit: &[u8]) -> Vec<u8> {
    std::iter::once(FRAME_TAG)
        .chain(id.to_be_bytes())
        .chain(x.to_be_bytes())
        .chain(y.to_be_bytes())
        .chain(std::iter::once(0))
        .chain(unit.iter().copied())
        .collect()
}

fn shown(id: u32, window: (i32, i32)) -> Shown {
    Shown { id, window }
}

fn rates(hz: u32, painted_per_s: f64, changes_per_s: f64) -> Rates {
    Rates {
        hz,
        painted_per_s,
        skipped_per_s: 0.0,
        changes_per_s,
        units_per_s: changes_per_s,
    }
}

fn ids(values: &[u32]) -> BTreeSet<u32> {
    values.iter().copied().collect()
}

#[test]
fn no_payloads_are_an_empty_stream() {
    assert_eq!(stream(&[]), Stream::default());
}

#[test]
fn one_frame_is_shown_with_its_id_and_window() {
    assert_eq!(
        stream(&[frame(7, (141, -3), &[0, 0, 1, 0x65])]),
        Stream {
            shown: vec![shown(7, (141, -3))],
            h264: vec![0, 0, 1, 0x65],
        }
    );
}

#[test]
fn many_frames_concatenate_and_cursor_messages_are_skipped() {
    assert_eq!(
        stream(&[
            frame(1, (0, 0), &[1]),
            vec![CURSOR_TAG, 0, 0, 0, 0, 0, 0, 0, 0],
            frame(2, (0, 0), &[2, 3]),
        ]),
        Stream {
            shown: vec![shown(1, (0, 0)), shown(2, (0, 0))],
            h264: vec![1, 2, 3],
        }
    );
}

#[test]
fn a_frame_without_a_unit_is_skipped() {
    assert_eq!(stream(&[frame(1, (0, 0), &[])]), Stream::default());
}

#[test]
fn the_target_is_right_of_the_centre_on_the_centre_row() {
    assert_eq!(target((1254, 1414)), (783, 707));
}

#[test]
fn one_window_puts_the_target_in_its_coordinates() {
    assert_eq!(
        sample_at(&[shown(1, (141, 0)), shown(2, (141, 0))], (783, 707)),
        Ok((642, 707))
    );
}

#[test]
fn no_frames_have_no_sample() {
    assert!(sample_at(&[], (783, 707)).is_err());
}

#[test]
fn a_moved_window_has_no_sample() {
    assert!(sample_at(&[shown(1, (141, 0)), shown(2, (140, 0))], (783, 707)).is_err());
}

#[test]
fn a_target_left_of_the_window_has_no_sample() {
    assert!(sample_at(&[shown(1, (800, 0))], (783, 707)).is_err());
}

#[test]
fn a_target_above_the_window_has_no_sample() {
    assert!(sample_at(&[shown(1, (0, 708))], (783, 707)).is_err());
}

#[test]
fn frames_and_levels_of_different_counts_are_an_error() {
    assert!(changed(&[shown(1, (0, 0))], &[16, 48]).is_err());
}

#[test]
fn changed_frames_are_named_by_id() {
    assert_eq!(
        changed(
            &[shown(4, (0, 0)), shown(5, (0, 0)), shown(6, (0, 0))],
            &[16, 16, 48]
        ),
        Ok(ids(&[6]))
    );
}

#[test]
fn only_acked_changes_count_per_second() {
    assert_eq!(
        per_second(&ids(&[2, 3, 4, 9]), &[1, 2, 3, 4], Duration::from_secs(2)),
        1.5
    );
}

#[test]
fn the_expected_rate_is_what_was_painted_below_the_refresh() {
    assert_eq!(expected(&rates(20, 19.9, 0.0), 59.95), 19.9);
}

#[test]
fn the_expected_rate_is_the_refresh_above_it() {
    assert_eq!(expected(&rates(125, 124.0, 0.0), 59.95), 59.95);
}

#[test]
fn nothing_painted_expects_no_change() {
    assert!(check(&rates(0, 0.0, 0.0), 59.95).ok);
}

#[test]
fn any_change_while_nothing_is_painted_fails() {
    assert!(!check(&rates(0, 0.0, 0.1), 59.95).ok);
}

#[test]
fn changes_within_ten_percent_pass() {
    assert_eq!(
        [
            check(&rates(20, 20.0, 18.0), 59.95).ok,
            check(&rates(20, 20.0, 22.0), 59.95).ok
        ],
        [true, true]
    );
}

#[test]
fn changes_beyond_ten_percent_fail() {
    assert_eq!(
        [
            check(&rates(20, 20.0, 17.9), 59.95).ok,
            check(&rates(20, 20.0, 22.1), 59.95).ok
        ],
        [false, false]
    );
}

#[test]
fn the_check_names_its_rate_and_expectation() {
    assert_eq!(
        check(&rates(125, 124.0, 57.0), 60.0).detail,
        "57.0 changes/s seen, expected min(painted 124.0/s, refresh 60.00 Hz) = 60.0 ± 10%"
    );
}

#[test]
fn the_check_is_named_by_its_rate() {
    assert_eq!(check(&rates(125, 124.0, 57.0), 60.0).gate, "content-125hz");
}

#[test]
fn a_row_shows_the_rates_to_one_decimal() {
    assert_eq!(
        row("lan", &rates(20, 19.96, 19.84)),
        "| lan | 20 | 20.0 (0.0) | 19.8 | 19.8 |"
    );
}
