use super::{BAND_ROWS, band, changes, level, next_due, period};
use std::time::Duration;

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

#[test]
fn the_first_frame_is_the_darkest_level() {
    assert_eq!(level(0), 16);
}

#[test]
fn the_eighth_frame_is_the_brightest_level() {
    assert_eq!(level(7), 240);
}

#[test]
fn the_levels_repeat_after_eight_frames() {
    assert_eq!(level(8), 16);
}

#[test]
fn zero_hz_has_no_period() {
    assert_eq!(period(0), None);
}

#[test]
fn one_hz_repaints_every_second() {
    assert_eq!(period(1), Some(Duration::from_secs(1)));
}

#[test]
fn sixty_hz_repaints_every_16_667_us() {
    assert_eq!(period(60), Some(Duration::from_nanos(16_666_666)));
}

#[test]
fn a_frame_on_time_is_due_one_period_later() {
    assert_eq!(next_due(ms(100), ms(10), ms(100)), ms(110));
}

#[test]
fn a_frame_late_by_less_than_a_period_keeps_the_schedule() {
    assert_eq!(next_due(ms(100), ms(10), ms(109)), ms(110));
}

#[test]
fn a_frame_late_by_several_periods_skips_the_missed_slots() {
    assert_eq!(next_due(ms(100), ms(10), ms(135)), ms(140));
}

#[test]
fn a_frame_late_by_exactly_one_period_is_due_at_the_slot_after() {
    assert_eq!(next_due(ms(100), ms(10), ms(110)), ms(120));
}

#[test]
fn the_band_is_centred_on_a_tall_output() {
    assert_eq!(band(1414, 707), (707 - BAND_ROWS / 2, BAND_ROWS));
}

#[test]
fn the_band_fills_an_output_shorter_than_itself() {
    assert_eq!(band(100, 50), (0, 100));
}

#[test]
fn the_band_stops_at_the_top() {
    assert_eq!(band(1414, 10), (0, BAND_ROWS));
}

#[test]
fn the_band_stops_at_the_bottom() {
    assert_eq!(band(1414, 1410), (1414 - BAND_ROWS, BAND_ROWS));
}

#[test]
fn no_levels_have_no_changes() {
    assert_eq!(changes(&[]), Vec::<bool>::new());
}

#[test]
fn one_level_is_no_change() {
    assert_eq!(changes(&[16]), vec![false]);
}

#[test]
fn many_levels_change_where_the_level_steps() {
    assert_eq!(
        changes(&[16, 16, 48, 48, 16, 80]),
        vec![false, false, true, false, true, true]
    );
}

#[test]
fn coding_noise_of_half_a_step_is_no_change() {
    assert_eq!(changes(&[16, 32, 16]), vec![false, false, false]);
}

#[test]
fn more_than_half_a_step_is_a_change() {
    assert_eq!(changes(&[16, 33]), vec![false, true]);
}
