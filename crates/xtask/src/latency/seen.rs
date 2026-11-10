use super::content;
use crate::report::Check;
use crate::viewer::{FRAME_HEADER, FRAME_TAG};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

pub const TOLERANCE: f64 = 0.1;
const SAMPLE_SIDE: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    pub id: u32,
    pub window: (i32, i32),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Stream {
    pub shown: Vec<Shown>,
    pub h264: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rates {
    pub hz: u32,
    pub painted_per_s: f64,
    pub skipped_per_s: f64,
    pub changes_per_s: f64,
    pub units_per_s: f64,
}

fn word(payload: &[u8], at: usize) -> Option<[u8; 4]> {
    payload.get(at..at + 4)?.try_into().ok()
}

pub fn stream(binary: &[Vec<u8>]) -> Stream {
    binary
        .iter()
        .filter_map(
            |payload: &Vec<u8>| match (payload.first(), payload.get(FRAME_HEADER..)) {
                (Some(&FRAME_TAG), Some(unit)) if !unit.is_empty() => Some((
                    Shown {
                        id: u32::from_be_bytes(word(payload, 1)?),
                        window: (
                            i32::from_be_bytes(word(payload, 5)?),
                            i32::from_be_bytes(word(payload, 9)?),
                        ),
                    },
                    unit,
                )),
                (None | Some(_), None | Some(_)) => None,
            },
        )
        .fold(
            Stream::default(),
            |mut stream: Stream, (shown, unit): (Shown, &[u8])| {
                stream.shown.push(shown);
                stream.h264.extend_from_slice(unit);
                stream
            },
        )
}

pub fn target((width, height): (u32, u32)) -> (u32, u32) {
    (width / 2 + width / 8, height / 2)
}

pub fn sample_at(shown: &[Shown], target: (u32, u32)) -> Result<(u32, u32), String> {
    let windows: BTreeSet<(i32, i32)> = shown.iter().map(|s: &Shown| s.window).collect();
    match windows.iter().collect::<Vec<&(i32, i32)>>().as_slice() {
        [(x, y)] => Ok((
            u32::try_from(i64::from(target.0) - i64::from(*x))
                .map_err(|_| format!("the sample {target:?} is left of the window at {x}"))?,
            u32::try_from(i64::from(target.1) - i64::from(*y))
                .map_err(|_| format!("the sample {target:?} is above the window at {y}"))?,
        )),
        [] => Err(String::from("the viewer saw no frame")),
        [_, _, ..] => Err(format!(
            "the encoded window moved during the run: {windows:?}"
        )),
    }
}

pub fn decode(h264: &Path, (x, y): (u32, u32)) -> Result<Vec<u8>, String> {
    let output: Output = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-f", "h264", "-i"])
        .arg(h264)
        .args([
            "-vf",
            &format!("crop={SAMPLE_SIDE}:{SAMPLE_SIDE}:{x}:{y},scale=1:1"),
            "-fps_mode",
            "passthrough",
            "-pix_fmt",
            "gray",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .map_err(|e| format!("cannot run ffmpeg: {e}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(format!(
            "ffmpeg {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

pub fn changed(shown: &[Shown], levels: &[u8]) -> Result<BTreeSet<u32>, String> {
    if shown.len() != levels.len() {
        return Err(format!(
            "{} frames received but {} decoded",
            shown.len(),
            levels.len()
        ));
    }
    Ok(shown
        .iter()
        .zip(content::changes(levels))
        .filter(|(_, change): &(&Shown, bool)| *change)
        .map(|(shown, _): (&Shown, bool)| shown.id)
        .collect())
}

pub fn per_second(changed: &BTreeSet<u32>, acked: &[u32], over: Duration) -> f64 {
    acked
        .iter()
        .filter(|id: &&u32| changed.contains(id))
        .count() as f64
        / over.as_secs_f64()
}

pub fn expected(rates: &Rates, refresh_hz: f64) -> f64 {
    rates.painted_per_s.min(refresh_hz)
}

pub fn check(rates: &Rates, refresh_hz: f64) -> Check {
    let expected: f64 = expected(rates, refresh_hz);
    Check {
        gate: format!("content-{}hz", rates.hz),
        ok: (rates.changes_per_s - expected).abs() <= TOLERANCE * expected,
        detail: format!(
            "{:.1} changes/s seen, expected min(painted {:.1}/s, refresh {refresh_hz:.2} Hz) = {expected:.1} ± {:.0}%",
            rates.changes_per_s,
            rates.painted_per_s,
            TOLERANCE * 100.0
        ),
    }
}

pub const HEADER: &str = "| profile | content Hz | painted/s (skipped/s) | changes/s seen | units/s |\n|---|---|---|---|---|";

pub fn row(profile: &str, rates: &Rates) -> String {
    format!(
        "| {profile} | {} | {:.1} ({:.1}) | {:.1} | {:.1} |",
        rates.hz, rates.painted_per_s, rates.skipped_per_s, rates.changes_per_s, rates.units_per_s
    )
}

#[cfg(test)]
mod tests;
