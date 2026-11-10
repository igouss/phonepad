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
