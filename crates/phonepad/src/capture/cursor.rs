use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use phonepad_core::geometry::Point;
use tokio::sync::watch;

use crate::hypr;

const CURSOR_SAMPLE_MOVING: Duration = Duration::from_millis(16);
const CURSOR_SAMPLE_STILL: Duration = Duration::from_millis(50);
const CURSOR_SETTLE: Duration = Duration::from_secs(1);

fn sample_period(still_for: Duration) -> Duration {
    if still_for < CURSOR_SETTLE {
        CURSOR_SAMPLE_MOVING
    } else {
        CURSOR_SAMPLE_STILL
    }
}

pub(super) fn sample_cursor(stop: &AtomicBool, cursor: &watch::Sender<Option<Point>>) {
    let mut failing: bool = false;
    let mut moved_at: Instant = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        match hypr::cursor() {
            Ok(at) => {
                if cursor.send_if_modified(|seen: &mut Option<Point>| {
                    let moved: bool = *seen != Some(at);
                    *seen = Some(at);
                    moved
                }) {
                    moved_at = Instant::now();
                }
                failing = false;
            }
            Err(err) => {
                if !failing {
                    eprintln!("cursor sample: {err:#}");
                }
                failing = true;
            }
        }
        thread::sleep(sample_period(moved_at.elapsed()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_that_just_moved_is_sampled_at_frame_rate() {
        assert_eq!(sample_period(Duration::ZERO), CURSOR_SAMPLE_MOVING);
    }

    #[test]
    fn cursor_still_just_under_the_settle_time_is_sampled_at_frame_rate() {
        assert_eq!(
            sample_period(CURSOR_SETTLE - Duration::from_millis(1)),
            CURSOR_SAMPLE_MOVING
        );
    }

    #[test]
    fn settled_cursor_is_sampled_slowly() {
        assert_eq!(sample_period(CURSOR_SETTLE), CURSOR_SAMPLE_STILL);
    }
}
