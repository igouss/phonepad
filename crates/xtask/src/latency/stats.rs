use super::link::Profile;
use crate::report::Check;
use std::time::Duration;

pub const STALL_MS: f64 = 250.0;
pub const RTT_TOLERANCE_MS: f64 = 15.0;
const PREFIX: &str = "timing ";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sample {
    Unit {
        id: u32,
        bytes: u64,
        capture_send: f64,
        send_ack: f64,
    },
    Drawn {
        decode: f64,
        draw: f64,
        gap: Option<f64>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spread {
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub units_per_s: f64,
    pub mbit_per_s: f64,
    pub drawn_per_s: f64,
    pub stalls: usize,
    pub longest_gap: Option<f64>,
    pub capture_send: Option<Spread>,
    pub send_ack: Option<Spread>,
    pub capture_ack: Option<Spread>,
    pub decode: Option<Spread>,
    pub draw: Option<Spread>,
}

pub fn sample(line: &str) -> Option<Sample> {
    let record: serde_json::Value = serde_json::from_str(line.strip_prefix(PREFIX)?).ok()?;
    let number = |key: &str| record[key].as_f64();
    match record["t"].as_str()? {
        "unit" => Some(Sample::Unit {
            id: u32::try_from(record["id"].as_u64()?).ok()?,
            bytes: record["bytes"].as_u64()?,
            capture_send: number("capture_send_ms")?,
            send_ack: number("send_ack_ms")?,
        }),
        "drawn" => Some(Sample::Drawn {
            decode: number("decode_ms")?,
            draw: number("draw_ms")?,
            gap: number("gap_ms"),
        }),
        _ => None,
    }
}

pub fn spread(mut values: Vec<f64>) -> Option<Spread> {
    values.sort_by(f64::total_cmp);
    let rank = |fraction: f64| -> Option<f64> {
        let index: usize = ((fraction * values.len() as f64).ceil() as usize).max(1) - 1;
        values.get(index).copied()
    };
    Some(Spread {
        p50: rank(0.5)?,
        p95: rank(0.95)?,
        max: *values.last()?,
    })
}

fn units(samples: &[Sample]) -> impl Iterator<Item = (u64, f64, f64)> + '_ {
    samples.iter().filter_map(|sample: &Sample| match *sample {
        Sample::Unit {
            bytes,
            capture_send,
            send_ack,
            ..
        } => Some((bytes, capture_send, send_ack)),
        Sample::Drawn { .. } => None,
    })
}

fn drawn(samples: &[Sample]) -> impl Iterator<Item = (f64, f64, Option<f64>)> + '_ {
    samples.iter().filter_map(|sample: &Sample| match *sample {
        Sample::Drawn { decode, draw, gap } => Some((decode, draw, gap)),
        Sample::Unit { .. } => None,
    })
}

pub fn summarize(samples: &[Sample], over: Duration) -> Summary {
    let seconds: f64 = over.as_secs_f64();
    let gaps: Vec<f64> = drawn(samples).filter_map(|(_, _, gap)| gap).collect();
    Summary {
        units_per_s: units(samples).count() as f64 / seconds,
        mbit_per_s: units(samples).map(|(bytes, _, _)| bytes).sum::<u64>() as f64 * 8.0
            / seconds
            / 1e6,
        drawn_per_s: drawn(samples).count() as f64 / seconds,
        stalls: gaps.iter().filter(|gap: &&f64| **gap > STALL_MS).count(),
        longest_gap: gaps.iter().copied().reduce(f64::max),
        capture_send: spread(units(samples).map(|(_, cs, _)| cs).collect()),
        send_ack: spread(units(samples).map(|(_, _, sa)| sa).collect()),
        capture_ack: spread(units(samples).map(|(_, cs, sa)| cs + sa).collect()),
        decode: spread(drawn(samples).map(|(decode, _, _)| decode).collect()),
        draw: spread(drawn(samples).map(|(_, draw, _)| draw).collect()),
    }
}

fn cell(spread: Option<Spread>) -> String {
    spread.map_or_else(
        || String::from("-"),
        |s: Spread| format!("{:.0} / {:.0} / {:.0}", s.p50, s.p95, s.max),
    )
}

fn link(profile: &Profile) -> String {
    format!(
        "{} ms ±{} ms, {}, {}% loss",
        profile.rtt.as_millis(),
        profile.jitter.as_millis(),
        profile.rate.map_or_else(
            || String::from("uncapped"),
            |rate| format!("{} Mbit/s", rate.get() as f64 / 1e6)
        ),
        profile.loss_ppm as f64 / 1e4
    )
}

pub const HEADER: &str = "| profile | RTT, jitter, cap, loss | units/s | Mbit/s | drawn/s | stalls > 250 ms (longest gap ms) | capture→send ms p50 / p95 / max | send→ack ms | capture→ack ms | receive→decode ms | decode→draw ms |\n|---|---|---|---|---|---|---|---|---|---|---|";

pub fn row(profile: &Profile, summary: &Summary) -> String {
    format!(
        "| {} | {} | {:.1} | {:.2} | {:.1} | {} ({}) | {} | {} | {} | {} | {} |",
        profile.name,
        link(profile),
        summary.units_per_s,
        summary.mbit_per_s,
        summary.drawn_per_s,
        summary.stalls,
        summary
            .longest_gap
            .map_or_else(|| String::from("-"), |gap: f64| format!("{gap:.0}")),
        cell(summary.capture_send),
        cell(summary.send_ack),
        cell(summary.capture_ack),
        cell(summary.decode),
        cell(summary.draw),
    )
}

pub fn added_rtt_check(added_ms: f64, base: &Summary, with: &Summary) -> Check {
    let medians: Option<(f64, f64)> = base
        .send_ack
        .zip(with.send_ack)
        .map(|(b, w): (Spread, Spread)| (b.p50, w.p50));
    Check {
        gate: format!("added-rtt-{added_ms:.0}"),
        ok: medians.is_some_and(|(b, w): (f64, f64)| {
            (w - b - added_ms).abs() <= RTT_TOLERANCE_MS
        }),
        detail: medians.map_or_else(
            || String::from("no acked units to compare"),
            |(b, w): (f64, f64)| {
                format!(
                    "send→ack p50 {w:.1} ms vs {b:.1} ms at no added RTT: +{:.1} ms, expected +{added_ms:.0} ± {RTT_TOLERANCE_MS:.0}",
                    w - b
                )
            },
        ),
    }
}

pub fn cap_check(
    gate: &str,
    measured_mbit: f64,
    floor_mbit: Option<f64>,
    ceiling_mbit: f64,
) -> Check {
    Check {
        gate: gate.to_owned(),
        ok: measured_mbit <= ceiling_mbit
            && floor_mbit.is_none_or(|floor: f64| measured_mbit >= floor),
        detail: format!(
            "{measured_mbit:.3} Mbit/s, expected {}at most {ceiling_mbit} Mbit/s",
            floor_mbit.map_or_else(String::new, |floor: f64| format!("at least {floor} and "))
        ),
    }
}

#[cfg(test)]
mod tests;
