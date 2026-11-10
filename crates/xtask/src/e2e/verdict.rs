use crate::report::Check;
use crate::testbed::Shutdown;
use phonepad_h264::nal;
use std::collections::BTreeSet;
use std::time::Duration;

pub const HEADER: usize = 9;
pub const FRAME_HEADER: usize = 14;
pub const FRAME_TAG: u8 = 0;
pub const CURSOR_TAG: u8 = 1;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub units: usize,
    pub keys: usize,
    pub malformed: usize,
    pub cursors: usize,
    pub first_unit_key: Option<bool>,
    pub stream: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Decoded {
    pub frames: usize,
    pub distinct: usize,
    pub errors: String,
}

pub fn tally(binary: &[Vec<u8>], texts: usize) -> Tally {
    binary.iter().fold(
        Tally {
            malformed: texts,
            ..Tally::default()
        },
        |mut tally: Tally, payload: &Vec<u8>| {
            match (
                payload.first(),
                payload.get(HEADER..),
                payload.get(FRAME_HEADER..),
            ) {
                (Some(&CURSOR_TAG), Some([]), _) => tally.cursors += 1,
                (Some(&FRAME_TAG), _, Some(unit)) if !unit.is_empty() => {
                    let key: bool = nal::contains_idr(unit);
                    tally.first_unit_key.get_or_insert(key);
                    tally.units += 1;
                    tally.keys += usize::from(key);
                    tally.stream.extend_from_slice(unit);
                }
                (None | Some(_), None | Some(_), None | Some(_)) => tally.malformed += 1,
            }
            tally
        },
    )
}

pub fn frame_hashes(framemd5: &str) -> Vec<&str> {
    framemd5
        .lines()
        .filter(|line: &&str| !line.trim().is_empty() && !line.starts_with('#'))
        .filter_map(|line: &str| line.rsplit(',').next())
        .map(str::trim)
        .collect()
}

pub fn distinct(hashes: &[&str]) -> usize {
    hashes.iter().collect::<BTreeSet<&&str>>().len()
}

pub fn viewer_checks(
    label: &str,
    tally: &Tally,
    decoded: &Decoded,
    watched: Duration,
) -> [Check; 4] {
    let rate: f64 = tally.units as f64 / watched.as_secs_f64();
    [
        Check {
            gate: format!("{label} units"),
            ok: tally.units > 0 && tally.malformed == 0,
            detail: format!(
                "{} units ({rate:.1}/s, {} keyframes, {} bytes), {} pointer messages, {} malformed messages",
                tally.units,
                tally.keys,
                tally.stream.len(),
                tally.cursors,
                tally.malformed
            ),
        },
        Check {
            gate: format!("{label} first-unit-key"),
            ok: tally.first_unit_key == Some(true),
            detail: format!(
                "first unit carries an IDR slice: {:?}",
                tally.first_unit_key
            ),
        },
        Check {
            gate: format!("{label} decodes"),
            ok: decoded.errors.is_empty() && decoded.frames > 0 && decoded.frames == tally.units,
            detail: format!(
                "ffmpeg decoded {} frames from {} units; errors: {:?}",
                decoded.frames, tally.units, decoded.errors
            ),
        },
        Check {
            gate: format!("{label} distinct"),
            ok: decoded.distinct > 0,
            detail: format!(
                "{} distinct of {} decoded frames",
                decoded.distinct, decoded.frames
            ),
        },
    ]
}

pub fn status_check(gate: &str, refused: Result<u16, String>) -> Check {
    Check {
        gate: gate.to_owned(),
        ok: refused == Ok(403),
        detail: match refused {
            Ok(status) => format!("HTTP {status}, expected 403"),
            Err(error) => format!("{error}, expected HTTP 403"),
        },
    }
}

pub fn shutdown_check(shutdown: &Shutdown, limit: Duration) -> Check {
    let (ok, detail): (bool, String) = match shutdown {
        Shutdown::Exited { after, code } => (
            *after <= limit && *code == Some(0),
            format!("exited {after:?} after SIGTERM with code {code:?}; limit {limit:?}, code 0"),
        ),
        Shutdown::Hung { waited } => (
            false,
            format!("still running {waited:?} after SIGTERM, killed; limit {limit:?}"),
        ),
        Shutdown::Early { code } => (false, format!("exited with code {code:?} before SIGTERM")),
    };
    Check {
        gate: String::from("sigterm-exit"),
        ok,
        detail,
    }
}

#[cfg(test)]
mod tests;
