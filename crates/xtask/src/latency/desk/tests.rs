use super::per_second;
use std::time::Duration;

#[test]
fn nothing_counted_is_zero_per_second() {
    assert_eq!(per_second(0, Duration::from_secs(2)), 0.0);
}

#[test]
fn counts_divide_by_the_seconds_measured() {
    assert_eq!(per_second(250, Duration::from_secs(4)), 62.5);
}
