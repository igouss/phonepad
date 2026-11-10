use std::time::{Duration, Instant};

use phonepad_core::geometry::Point;

use crate::hypr;

const CURSOR_SAMPLE_MOVING: Duration = Duration::from_micros(16_667);
const CURSOR_SAMPLE_STILL: Duration = Duration::from_millis(50);
const CURSOR_SETTLE: Duration = Duration::from_secs(1);

fn sample_period(still_for: Duration) -> Duration {
    if still_for < CURSOR_SETTLE {
        CURSOR_SAMPLE_MOVING
    } else {
        CURSOR_SAMPLE_STILL
    }
}

fn until_due(sampled_at: Option<Instant>, moved_at: Instant, now: Instant) -> Duration {
    sampled_at.map_or(Duration::ZERO, |sampled: Instant| {
        (sampled + sample_period(now.saturating_duration_since(moved_at)))
            .saturating_duration_since(now)
    })
}

pub(super) struct Sampler {
    at: Option<Point>,
    sampled_at: Option<Instant>,
    moved_at: Instant,
    failing: bool,
}

impl Sampler {
    pub(super) fn new(now: Instant) -> Sampler {
        Sampler {
            at: None,
            sampled_at: None,
            moved_at: now,
            failing: false,
        }
    }

    pub(super) fn at(&self) -> Option<Point> {
        self.at
    }

    pub(super) fn until_due(&self, now: Instant) -> Duration {
        until_due(self.sampled_at, self.moved_at, now)
    }

    pub(super) fn sample_if_due(&mut self, now: Instant) -> bool {
        if !self.until_due(now).is_zero() {
            return false;
        }
        self.record(now, hypr::cursor())
    }

    fn record(&mut self, now: Instant, reading: anyhow::Result<Point>) -> bool {
        self.sampled_at = Some(now);
        match reading {
            Ok(at) => {
                self.failing = false;
                let moved: bool = self.at != Some(at);
                if moved {
                    self.at = Some(at);
                    self.moved_at = now;
                }
                moved
            }
            Err(err) => {
                if !self.failing {
                    eprintln!("cursor sample: {err:#}");
                }
                self.failing = true;
                false
            }
        }
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

    #[test]
    fn a_cursor_never_sampled_is_due_now() {
        let now: Instant = Instant::now();
        assert_eq!(until_due(None, now, now), Duration::ZERO);
    }

    #[test]
    fn a_moving_cursor_is_due_one_frame_after_its_sample() {
        let now: Instant = Instant::now();
        assert_eq!(
            until_due(Some(now), now, now + Duration::from_millis(10)),
            CURSOR_SAMPLE_MOVING - Duration::from_millis(10)
        );
    }

    #[test]
    fn a_settled_cursor_is_due_after_the_slow_period() {
        let now: Instant = Instant::now();
        let sampled: Instant = now + CURSOR_SETTLE;
        assert_eq!(
            until_due(Some(sampled), now, sampled + Duration::from_millis(10)),
            CURSOR_SAMPLE_STILL - Duration::from_millis(10)
        );
    }

    const HERE: Point = Point { x: 10, y: 20 };
    const THERE: Point = Point { x: 11, y: 20 };

    #[test]
    fn a_new_sampler_knows_no_position_and_is_due() {
        let now: Instant = Instant::now();
        let sampler: Sampler = Sampler::new(now);
        assert_eq!(
            (sampler.at(), sampler.until_due(now)),
            (None, Duration::ZERO)
        );
    }

    #[test]
    fn the_first_reading_is_a_move_and_waits_one_frame() {
        let now: Instant = Instant::now();
        let mut sampler: Sampler = Sampler::new(now);
        assert_eq!(
            (
                sampler.record(now, Ok(HERE)),
                sampler.at(),
                sampler.until_due(now)
            ),
            (true, Some(HERE), CURSOR_SAMPLE_MOVING)
        );
    }

    #[test]
    fn the_same_reading_again_is_not_a_move() {
        let now: Instant = Instant::now();
        let mut sampler: Sampler = Sampler::new(now);
        sampler.record(now, Ok(HERE));
        assert!(!sampler.record(now + CURSOR_SETTLE, Ok(HERE)));
    }

    #[test]
    fn a_reading_unchanged_past_the_settle_time_slows_sampling() {
        let now: Instant = Instant::now();
        let mut sampler: Sampler = Sampler::new(now);
        sampler.record(now, Ok(HERE));
        let later: Instant = now + CURSOR_SETTLE;
        sampler.record(later, Ok(HERE));
        assert_eq!(sampler.until_due(later), CURSOR_SAMPLE_STILL);
    }

    #[test]
    fn a_different_reading_is_a_move_and_restarts_fast_sampling() {
        let now: Instant = Instant::now();
        let mut sampler: Sampler = Sampler::new(now);
        sampler.record(now, Ok(HERE));
        let later: Instant = now + CURSOR_SETTLE;
        assert_eq!(
            (
                sampler.record(later, Ok(THERE)),
                sampler.at(),
                sampler.until_due(later)
            ),
            (true, Some(THERE), CURSOR_SAMPLE_MOVING)
        );
    }

    #[test]
    fn a_failed_reading_keeps_the_last_position_and_is_not_a_move() {
        let now: Instant = Instant::now();
        let mut sampler: Sampler = Sampler::new(now);
        sampler.record(now, Ok(HERE));
        assert_eq!(
            (
                sampler.record(now, Err(anyhow::anyhow!("socket gone"))),
                sampler.at()
            ),
            (false, Some(HERE))
        );
    }

    #[test]
    fn a_sampler_not_yet_due_does_not_sample() {
        let now: Instant = Instant::now();
        let mut sampler: Sampler = Sampler::new(now);
        sampler.record(now, Ok(HERE));
        assert!(!sampler.sample_if_due(now));
    }

    #[test]
    fn an_overdue_sample_is_due_now() {
        let now: Instant = Instant::now();
        assert_eq!(
            until_due(Some(now), now, now + Duration::from_secs(5)),
            Duration::ZERO
        );
    }
}
