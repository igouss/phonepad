use std::time::{Duration, Instant};

use phonepad_core::geometry::{Monitor, Point, monitor_under};

use super::Pointer;

use crate::hypr;
use crate::marks::{Applied, Marks};

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Sample {
    pub(super) at: Point,
    pub(super) applied: Applied,
}

impl Sample {
    pub(super) fn pointer(&self, monitors: &[Monitor]) -> Option<Pointer> {
        Some(Pointer {
            at: self.at,
            applied: self.applied.clone(),
            monitor: monitor_under(self.at, monitors)?.clone(),
        })
    }
}

pub(super) struct Sampler {
    at: Option<Sample>,
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

    pub(super) fn at(&self) -> Option<&Sample> {
        self.at.as_ref()
    }

    pub(super) fn until_due(&self, now: Instant) -> Duration {
        until_due(self.sampled_at, self.moved_at, now)
    }

    pub(super) fn sample_if_due(&mut self, now: Instant, marks: &Marks) -> bool {
        if !self.until_due(now).is_zero() {
            return false;
        }
        self.take(now, marks.read(hypr::cursor))
    }

    fn take(&mut self, now: Instant, read: Option<(Applied, anyhow::Result<Point>)>) -> bool {
        match read {
            Some((applied, reading)) => self.record(now, applied, reading),
            None => {
                self.sampled_at = Some(now);
                false
            }
        }
    }

    fn record(&mut self, now: Instant, applied: Applied, reading: anyhow::Result<Point>) -> bool {
        self.sampled_at = Some(now);
        match reading {
            Ok(at) => {
                self.failing = false;
                let sample: Sample = Sample { at, applied };
                let moved: bool = self.at.as_ref() != Some(&sample);
                if moved {
                    self.at = Some(sample);
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
    use crate::marks::PhoneId;

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

    fn still(at: Point) -> Sample {
        Sample {
            at,
            applied: Applied::new(),
        }
    }

    fn after(phone: u64, seq: u32) -> Applied {
        std::iter::once((PhoneId(phone), seq)).collect()
    }

    fn screen(name: &str, x: i32) -> Monitor {
        Monitor::from_mode(name.to_owned(), Point { x, y: 0 }, 100, 100, 1.0, 0).unwrap()
    }

    #[test]
    fn a_sample_with_no_monitor_is_no_pointer() {
        assert_eq!(still(HERE).pointer(&[]), None);
    }

    #[test]
    fn a_sample_becomes_a_pointer_on_the_monitor_under_it() {
        let sample: Sample = Sample {
            at: Point { x: 150, y: 20 },
            applied: after(1, 3),
        };
        assert_eq!(
            sample.pointer(&[screen("A", 0), screen("B", 100)]),
            Some(Pointer {
                at: Point { x: 150, y: 20 },
                applied: after(1, 3),
                monitor: screen("B", 100)
            })
        );
    }

    #[test]
    fn a_new_applied_move_at_the_same_position_is_a_change() {
        let now: Instant = Instant::now();
        let mut sampler: Sampler = Sampler::new(now);
        sampler.record(now, after(1, 3), Ok(HERE));
        assert_eq!(
            (
                sampler.record(now + CURSOR_SETTLE, after(1, 4), Ok(HERE)),
                sampler.at().map(|sample: &Sample| sample.applied.clone()),
                sampler.until_due(now + CURSOR_SETTLE)
            ),
            (true, Some(after(1, 4)), CURSOR_SAMPLE_MOVING)
        );
    }

    #[test]
    fn a_reading_a_move_overlapped_keeps_the_last_sample_and_waits_a_period() {
        let now: Instant = Instant::now();
        let mut sampler: Sampler = Sampler::new(now);
        sampler.record(now, after(1, 3), Ok(HERE));
        let later: Instant = now + Duration::from_millis(20);
        assert_eq!(
            (
                sampler.take(later, None),
                sampler.at().map(|sample: &Sample| sample.applied.clone()),
                sampler.until_due(later)
            ),
            (false, Some(after(1, 3)), CURSOR_SAMPLE_MOVING)
        );
    }

    #[test]
    fn a_clean_reading_is_recorded() {
        let now: Instant = Instant::now();
        let mut sampler: Sampler = Sampler::new(now);
        assert_eq!(
            (
                sampler.take(now, Some((after(1, 3), Ok(THERE)))),
                sampler.at().cloned()
            ),
            (
                true,
                Some(Sample {
                    at: THERE,
                    applied: after(1, 3)
                })
            )
        );
    }

    #[test]
    fn a_failed_reading_keeps_the_applied_moves_of_the_last_good_one() {
        let now: Instant = Instant::now();
        let mut sampler: Sampler = Sampler::new(now);
        sampler.record(now, after(1, 3), Ok(HERE));
        sampler.record(now, after(1, 4), Err(anyhow::anyhow!("socket gone")));
        assert_eq!(
            sampler.at().map(|sample: &Sample| sample.applied.clone()),
            Some(after(1, 3))
        );
    }

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
                sampler.record(now, Applied::new(), Ok(HERE)),
                sampler.at(),
                sampler.until_due(now)
            ),
            (true, Some(&still(HERE)), CURSOR_SAMPLE_MOVING)
        );
    }

    #[test]
    fn the_same_reading_again_is_not_a_move() {
        let now: Instant = Instant::now();
        let mut sampler: Sampler = Sampler::new(now);
        sampler.record(now, Applied::new(), Ok(HERE));
        assert!(!sampler.record(now + CURSOR_SETTLE, Applied::new(), Ok(HERE)));
    }

    #[test]
    fn a_reading_unchanged_past_the_settle_time_slows_sampling() {
        let now: Instant = Instant::now();
        let mut sampler: Sampler = Sampler::new(now);
        sampler.record(now, Applied::new(), Ok(HERE));
        let later: Instant = now + CURSOR_SETTLE;
        sampler.record(later, Applied::new(), Ok(HERE));
        assert_eq!(sampler.until_due(later), CURSOR_SAMPLE_STILL);
    }

    #[test]
    fn a_different_reading_is_a_move_and_restarts_fast_sampling() {
        let now: Instant = Instant::now();
        let mut sampler: Sampler = Sampler::new(now);
        sampler.record(now, Applied::new(), Ok(HERE));
        let later: Instant = now + CURSOR_SETTLE;
        assert_eq!(
            (
                sampler.record(later, Applied::new(), Ok(THERE)),
                sampler.at(),
                sampler.until_due(later)
            ),
            (true, Some(&still(THERE)), CURSOR_SAMPLE_MOVING)
        );
    }

    #[test]
    fn a_failed_reading_keeps_the_last_position_and_is_not_a_move() {
        let now: Instant = Instant::now();
        let mut sampler: Sampler = Sampler::new(now);
        sampler.record(now, Applied::new(), Ok(HERE));
        assert_eq!(
            (
                sampler.record(now, Applied::new(), Err(anyhow::anyhow!("socket gone"))),
                sampler.at()
            ),
            (false, Some(&still(HERE)))
        );
    }

    #[test]
    fn a_sampler_not_yet_due_does_not_sample() {
        let now: Instant = Instant::now();
        let mut sampler: Sampler = Sampler::new(now);
        sampler.record(now, Applied::new(), Ok(HERE));
        assert!(!sampler.sample_if_due(now, &Marks::default()));
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
