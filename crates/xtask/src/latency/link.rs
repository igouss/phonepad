use std::num::NonZeroU64;
use std::time::Duration;

pub const SEGMENT: usize = 1448;
pub const MIN_RTO: Duration = Duration::from_millis(200);
const PER_MILLION: u64 = 1_000_000;
const NANOS_PER_SECOND: u128 = 1_000_000_000;
const BITS_PER_BYTE: u128 = 8;
const MANTISSA: u32 = 53;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub rtt: Duration,
    pub jitter: Duration,
    pub rate: Option<NonZeroU64>,
    pub loss_ppm: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Draw {
    pub jitter: f64,
    pub lost: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lane {
    pub free_at: Duration,
    pub last_out: Duration,
}

fn millis(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

fn mbit(megabits: u64) -> Option<NonZeroU64> {
    NonZeroU64::new(megabits * PER_MILLION)
}

pub fn named(name: &str) -> Option<Profile> {
    let (rtt, jitter, rate, loss_ppm): (u64, u64, Option<NonZeroU64>, u64) = match name {
        "lan" => (0, 0, None, 0),
        "wifi" => (10, 0, mbit(50), 0),
        "lte" => (60, 20, mbit(10), 0),
        "bad-lte" => (150, 50, mbit(3), 10_000),
        _ => return None,
    };
    Some(Profile {
        name: name.to_owned(),
        rtt: millis(rtt),
        jitter: millis(jitter),
        rate,
        loss_ppm,
    })
}

fn number(key: &str, value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|parsed: &f64| parsed.is_finite() && *parsed >= 0.0)
        .ok_or_else(|| format!("{key} must be a non-negative number, got `{value}`"))
}

fn setting(mut profile: Profile, pair: &str) -> Result<Profile, String> {
    let (key, value): (&str, &str) = pair
        .split_once('=')
        .ok_or_else(|| format!("`{pair}` is not key=value"))?;
    let amount: f64 = number(key, value)?;
    match key {
        "rtt" => profile.rtt = Duration::from_secs_f64(amount / 1e3),
        "jitter" => profile.jitter = Duration::from_secs_f64(amount / 1e3),
        "rate" => {
            profile.rate = Some(
                NonZeroU64::new((amount * 1e6).round() as u64)
                    .ok_or_else(|| format!("rate must be above zero, got `{value}`"))?,
            )
        }
        "loss" if amount <= 100.0 => profile.loss_ppm = (amount * 1e4).round() as u64,
        "loss" => return Err(format!("loss is a percentage, got `{value}`")),
        _ => {
            return Err(format!(
                "unknown setting `{key}`; use rtt, jitter, rate or loss"
            ));
        }
    }
    Ok(profile)
}

pub fn parse(text: &str) -> Result<Profile, String> {
    match named(text) {
        Some(profile) => Ok(profile),
        None if text.contains('=') => text.split(',').try_fold(
            Profile {
                name: text.to_owned(),
                rtt: Duration::ZERO,
                jitter: Duration::ZERO,
                rate: None,
                loss_ppm: 0,
            },
            setting,
        ),
        None => Err(format!(
            "unknown profile `{text}`; use lan, wifi, lte, bad-lte or rtt=<ms>,jitter=<ms>,rate=<Mbit/s>,loss=<%>"
        )),
    }
}

pub fn serialization(bytes: usize, rate: Option<NonZeroU64>) -> Duration {
    rate.map_or(Duration::ZERO, |rate: NonZeroU64| {
        let nanos: u128 = bytes as u128 * BITS_PER_BYTE * NANOS_PER_SECOND / u128::from(rate.get());
        Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
    })
}

pub fn one_way(profile: &Profile, jitter: f64) -> Duration {
    Duration::from_secs_f64(
        (profile.rtt.as_secs_f64() + profile.jitter.as_secs_f64() * jitter.clamp(-1.0, 1.0))
            .max(0.0)
            / 2.0,
    )
}

pub fn retransmission(profile: &Profile) -> Duration {
    MIN_RTO.max(profile.rtt * 2)
}

pub fn schedule(
    lane: Lane,
    profile: &Profile,
    arrival: Duration,
    bytes: usize,
    draw: Draw,
) -> Lane {
    let free_at: Duration = arrival.max(lane.free_at) + serialization(bytes, profile.rate);
    let stall: Duration = if draw.lost {
        retransmission(profile)
    } else {
        Duration::ZERO
    };
    Lane {
        free_at,
        last_out: (free_at + one_way(profile, draw.jitter) + stall).max(lane.last_out),
    }
}

pub struct Rng(u64);

impl Rng {
    pub fn seeded(seed: u64) -> Rng {
        Rng(seed | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn draw(&mut self, loss_ppm: u64) -> Draw {
        let unit: f64 = (self.next() >> (u64::BITS - MANTISSA)) as f64 / (1u64 << MANTISSA) as f64;
        Draw {
            jitter: unit * 2.0 - 1.0,
            lost: self.next() % PER_MILLION < loss_ppm,
        }
    }
}

#[cfg(test)]
mod tests;
