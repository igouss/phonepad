use super::link::{self, Profile};
use super::nested::STOCK;
use std::path::PathBuf;
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 8771;
pub const DEFAULT_MEASURED: Duration = Duration::from_secs(15);
pub const DEFAULT_HZ: u32 = 60;

#[derive(Debug, PartialEq, Eq)]
pub enum Run {
    Page(Vec<Profile>),
    Viewer(Vec<Profile>),
    Touch(Vec<Profile>),
    Calibrate,
    Content,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Desktop {
    Live,
    Nested { hyprland: PathBuf, hz: u32 },
}

#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub run: Run,
    pub port: u16,
    pub measured: Duration,
    pub desktop: Desktop,
}

#[derive(Default)]
struct Words<'a> {
    viewer: bool,
    touch: bool,
    nested: bool,
    hyprland: Option<PathBuf>,
    hz: Option<u32>,
    port: Option<u16>,
    seconds: Option<u64>,
    rest: Vec<&'a str>,
}

fn option<T: std::str::FromStr>(key: &str, value: &str) -> Result<T, String> {
    value
        .parse::<T>()
        .map_err(|_| format!("{key}= needs a positive integer, got `{value}`"))
}

fn words<'a>(all: &[&'a str]) -> Result<Words<'a>, String> {
    all.iter()
        .try_fold(Words::default(), |mut words: Words<'a>, word: &&'a str| {
            match (word.split_once('='), *word) {
                (Some(("port", value)), _) => words.port = Some(option("port", value)?),
                (Some(("seconds", value)), _) => words.seconds = Some(option("seconds", value)?),
                (Some(("hyprland", value)), _) => words.hyprland = Some(PathBuf::from(value)),
                (Some(("hz", value)), _) => words.hz = Some(option("hz", value)?),
                (None, "viewer") => words.viewer = true,
                (None, "touch") => words.touch = true,
                (None, "nested") => words.nested = true,
                (Some(_) | None, _) => words.rest.push(word),
            }
            Ok(words)
        })
}

pub fn plan(all: &[&str]) -> Result<Plan, String> {
    let words: Words = words(all)?;
    let desktop: Desktop = match (words.nested, words.hyprland.clone(), words.hz) {
        (false, None, None) => Desktop::Live,
        (false, Some(_), _) | (false, None, Some(_)) => {
            return Err(String::from("hyprland= and hz= need nested"));
        }
        (true, hyprland, hz) => Desktop::Nested {
            hyprland: hyprland.unwrap_or_else(|| PathBuf::from(STOCK)),
            hz: hz.unwrap_or(DEFAULT_HZ),
        },
    };
    if words.touch && words.nested {
        return Err(String::from(
            "touch moves the live pointer through uinput, which a nested Hyprland does not see",
        ));
    }
    let run: Run = match (words.viewer, words.touch, words.rest.as_slice()) {
        (true, true, _) => {
            return Err(String::from(
                "touch drives the page, the acking viewer has none",
            ));
        }
        (_, _, []) => {
            return Err(String::from(
                "latency needs a profile: lan, wifi, lte, bad-lte, rtt=<ms>,jitter=<ms>,rate=<Mbit/s>,loss=<%>, or calibrate",
            ));
        }
        (false, false, ["calibrate"]) => match (&desktop, words.hz) {
            (Desktop::Live, _) => Run::Calibrate,
            (Desktop::Nested { .. }, None) => Run::Content,
            (Desktop::Nested { .. }, Some(_)) => {
                return Err(String::from(
                    "nested calibrate paints at its own rates; drop hz=",
                ));
            }
        },
        (true, false, ["calibrate"]) | (false, true, ["calibrate"]) => {
            return Err(String::from("calibrate always uses the acking viewer"));
        }
        (viewer, touch, profiles) => {
            let profiles: Vec<Profile> = profiles
                .iter()
                .map(|word: &&str| link::parse(word))
                .collect::<Result<Vec<Profile>, String>>()?;
            match (viewer, touch) {
                (true, _) => Run::Viewer(profiles),
                (false, true) => Run::Touch(profiles),
                (false, false) => Run::Page(profiles),
            }
        }
    };
    Ok(Plan {
        run,
        port: words.port.unwrap_or(DEFAULT_PORT),
        measured: match words.seconds {
            Some(0) => return Err(String::from("seconds= must be at least 1")),
            Some(seconds) => Duration::from_secs(seconds),
            None => DEFAULT_MEASURED,
        },
        desktop,
    })
}

#[cfg(test)]
mod tests;
