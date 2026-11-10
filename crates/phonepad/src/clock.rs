use std::time::Instant;

use rustix::time::{ClockId, Timespec, clock_gettime};

const NANOS_PER_SECOND: u64 = 1_000_000_000;

#[derive(Clone, Copy, Debug)]
pub struct Clock(Instant);

impl Clock {
    pub fn start() -> Clock {
        Clock(Instant::now())
    }

    pub fn now_ms(self) -> u64 {
        u64::try_from(self.0.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

pub fn monotonic_ns() -> u64 {
    let now: Timespec = clock_gettime(ClockId::Monotonic);
    u64::try_from(now.tv_sec)
        .unwrap_or(0)
        .saturating_mul(NANOS_PER_SECOND)
        .saturating_add(u64::try_from(now.tv_nsec).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::{Clock, monotonic_ns};
    use std::time::Duration;

    #[test]
    fn the_monotonic_clock_advances_by_the_time_slept() {
        let before: u64 = monotonic_ns();
        std::thread::sleep(Duration::from_millis(3));
        let after: u64 = monotonic_ns();
        assert!(after.saturating_sub(before) >= 3_000_000);
    }

    #[test]
    fn the_session_clock_counts_milliseconds_since_its_start() {
        let clock: Clock = Clock::start();
        std::thread::sleep(Duration::from_millis(3));
        assert!((3..1_000).contains(&clock.now_ms()));
    }
}
