use std::collections::VecDeque;

pub const MAX_UNITS: usize = 256;
pub const FLOOR_BYTES: u64 = 64 * 1024;
pub const CEILING_BYTES: u64 = 4 * 1024 * 1024;
pub const QUEUE_CAP_US: u64 = 60_000;
pub const MIN_RTT_WINDOW_US: u64 = 10_000_000;
pub const RATE_WINDOW_US: u64 = 2_000_000;
const MICROS_PER_SECOND: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sent {
    pub id: u32,
    pub bytes: u32,
    pub at_us: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Flying {
    id: u32,
    bytes: u32,
    sent_us: u64,
    delivered: u64,
    delivered_us: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Filtered {
    value: u64,
    at_us: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Occupancy {
    Empty,
    Room,
    Full,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pipe {
    flight: VecDeque<Flying>,
    delivered: u64,
    delivered_us: u64,
    estimate: Option<Estimate>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Estimate {
    min_rtt_us: Filtered,
    rate: Filtered,
}

fn kept(
    current: Option<Filtered>,
    sample: u64,
    at_us: u64,
    window_us: u64,
    keeps: fn(u64, u64) -> bool,
) -> Filtered {
    match current {
        Some(held)
            if keeps(held.value, sample) && at_us.saturating_sub(held.at_us) <= window_us =>
        {
            held
        }
        Some(_) | None => Filtered {
            value: sample,
            at_us,
        },
    }
}

fn lower(held: u64, sample: u64) -> bool {
    held <= sample
}

fn higher(held: u64, sample: u64) -> bool {
    held >= sample
}

impl Pipe {
    pub fn is_empty(&self) -> bool {
        self.flight.is_empty()
    }

    pub fn carries(&self, id: u32) -> bool {
        self.flight.iter().any(|flying: &Flying| flying.id == id)
    }

    pub fn ends_with(&self, id: u32) -> bool {
        self.flight
            .back()
            .is_some_and(|flying: &Flying| flying.id == id)
    }

    pub fn in_flight(&self) -> u64 {
        self.flight
            .iter()
            .map(|flying: &Flying| u64::from(flying.bytes))
            .sum()
    }

    pub fn window(&self) -> u64 {
        self.estimate.map_or(FLOOR_BYTES, |estimate: Estimate| {
            estimate
                .rate
                .value
                .saturating_mul(estimate.min_rtt_us.value.saturating_add(QUEUE_CAP_US))
                .div_euclid(MICROS_PER_SECOND)
                .clamp(FLOOR_BYTES, CEILING_BYTES)
        })
    }

    pub fn admits(&self, bytes: u32) -> bool {
        self.is_empty()
            || (self.flight.len() < MAX_UNITS
                && self.in_flight().saturating_add(u64::from(bytes)) <= self.window())
    }

    pub fn occupancy(&self) -> Occupancy {
        match (self.is_empty(), self.admits(1)) {
            (true, _) => Occupancy::Empty,
            (false, true) => Occupancy::Room,
            (false, false) => Occupancy::Full,
        }
    }

    pub fn sent(mut self, sent: Sent) -> Pipe {
        if self.is_empty() {
            self.delivered_us = sent.at_us;
        }
        self.flight.push_back(Flying {
            id: sent.id,
            bytes: sent.bytes,
            sent_us: sent.at_us,
            delivered: self.delivered,
            delivered_us: self.delivered_us,
        });
        self
    }

    pub fn acked(mut self, id: u32, at_us: u64) -> Pipe {
        let Some((position, acked)) = self
            .flight
            .iter()
            .enumerate()
            .find(|(_, flying): &(usize, &Flying)| flying.id == id)
            .map(|(position, flying): (usize, &Flying)| (position, *flying))
        else {
            return self;
        };
        let landed: u64 = self
            .flight
            .drain(..=position)
            .map(|flying: Flying| u64::from(flying.bytes))
            .sum();
        self.delivered = self.delivered.saturating_add(landed);
        self.delivered_us = at_us;
        let rate: u64 = self
            .delivered
            .saturating_sub(acked.delivered)
            .saturating_mul(MICROS_PER_SECOND)
            / at_us.saturating_sub(acked.delivered_us).max(1);
        let rtt_us: u64 = at_us.saturating_sub(acked.sent_us);
        self.estimate = Some(Estimate {
            min_rtt_us: kept(
                self.estimate.map(|estimate: Estimate| estimate.min_rtt_us),
                rtt_us,
                at_us,
                MIN_RTT_WINDOW_US,
                lower,
            ),
            rate: kept(
                self.estimate.map(|estimate: Estimate| estimate.rate),
                rate,
                at_us,
                RATE_WINDOW_US,
                higher,
            ),
        });
        self
    }
}

#[cfg(test)]
mod tests;
