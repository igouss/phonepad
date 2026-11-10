mod browser;
mod link;
mod mover;
mod proxy;
mod stats;

use crate::report::{Check, Report};
use crate::testbed::{self, Instance, Runtime, Scratch};
use crate::viewer::{self, Handshake};
use browser::{Browser, PHONE};
use link::Profile;
use mover::Mover;
use proxy::Emulator;
use stats::{Sample, Summary};
use std::net::{Ipv4Addr, TcpListener};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

const PORT: u16 = 8771;
const LOGIN: &str = "test@example.invalid";
const RUNTIME_NAME: &str = "pplat";
const TIMINGS: (&str, &str) = ("PHONEPAD_TIMINGS", "1");
const SETTLE: Duration = Duration::from_secs(3);
const MEASURED: Duration = Duration::from_secs(15);
const FIRST_UNIT_TIMEOUT: Duration = Duration::from_secs(20);
const VIEWER_VIEW: (u32, u32) = (648, 1072);
const ADDED_RTT_MS: [u64; 2] = [100, 200];
const CAP: &str = "rate=2";
const CAP_CEILING_MBIT: f64 = 2.1;
const CAP_FLOOR_MBIT: f64 = 1.9;
const BULK_BYTES: usize = 750_000;

#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    Profiles(Vec<Profile>),
    Calibrate,
}

pub fn plan(words: &[&str]) -> Result<Plan, String> {
    match words {
        [] => Err(String::from(
            "latency needs a profile: lan, wifi, lte, bad-lte, rtt=<ms>,jitter=<ms>,rate=<Mbit/s>,loss=<%>, or calibrate",
        )),
        ["calibrate"] => Ok(Plan::Calibrate),
        profiles => profiles
            .iter()
            .map(|word: &&str| link::parse(word))
            .collect::<Result<Vec<Profile>, String>>()
            .map(Plan::Profiles),
    }
}

fn calibration() -> Result<Vec<Profile>, String> {
    std::iter::once(String::from("rtt=0"))
        .chain(ADDED_RTT_MS.map(|ms: u64| format!("rtt={ms}")))
        .chain(std::iter::once(String::from(CAP)))
        .map(|text: String| link::parse(&text))
        .collect()
}

struct Collected {
    samples: Vec<Sample>,
    malformed: Vec<String>,
}

fn collect(lines: &Receiver<String>, over: Duration) -> Result<Collected, String> {
    let deadline: Instant = Instant::now() + over;
    let mut collected: Collected = Collected {
        samples: Vec::new(),
        malformed: Vec::new(),
    };
    loop {
        match lines.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(line) => match (line.starts_with("timing "), stats::sample(&line)) {
                (_, Some(sample)) => collected.samples.push(sample),
                (true, None) => collected.malformed.push(line),
                (false, None) => {}
            },
            Err(RecvTimeoutError::Timeout) => return Ok(collected),
            Err(RecvTimeoutError::Disconnected) => {
                return Err(String::from("the test instance closed its stdout"));
            }
        }
    }
}

fn first_unit(lines: &Receiver<String>) -> Result<Duration, String> {
    let start: Instant = Instant::now();
    loop {
        let left: Duration = FIRST_UNIT_TIMEOUT.saturating_sub(start.elapsed());
        match lines
            .recv_timeout(left)
            .map(|line: String| stats::sample(&line))
        {
            Ok(Some(Sample::Unit { .. })) => return Ok(start.elapsed()),
            Ok(Some(Sample::Drawn { .. }) | None) => {}
            Err(_) => return Err(format!("no acked unit within {FIRST_UNIT_TIMEOUT:?}")),
        }
    }
}

fn measure(
    report: &mut Report,
    instance: &Instance,
    emulator: &Emulator,
    phases: &[Profile],
) -> Result<Vec<Summary>, String> {
    phases
        .iter()
        .map(|profile: &Profile| {
            emulator.set(profile.clone());
            collect(instance.lines(), SETTLE)?;
            let collected: Collected = collect(instance.lines(), MEASURED)?;
            report.line(
                &format!("{} records", profile.name),
                collected.malformed.is_empty() && !collected.samples.is_empty(),
                format!(
                    "{} timing records, {} malformed{}",
                    collected.samples.len(),
                    collected.malformed.len(),
                    collected
                        .malformed
                        .first()
                        .map_or_else(String::new, |line: &String| format!(", first: {line}"))
                ),
            );
            Ok(stats::summarize(&collected.samples, MEASURED))
        })
        .collect()
}

fn page(scratch: &Scratch, emulator: &Emulator) -> Result<Option<Browser>, String> {
    let mut browser: Browser = Browser::launch(&scratch.dir.join("chromium"))?;
    browser.open(&format!("http://127.0.0.1:{}/", emulator.port), LOGIN)?;
    println!(
        "INFO page {}x{} CSS px at {}x through the emulator on port {}: {}",
        PHONE.width,
        PHONE.height,
        PHONE.scale,
        emulator.port,
        browser.view()?
    );
    Ok(Some(browser))
}

fn viewer(emulator: &Emulator, phases: usize) -> Result<Option<Browser>, String> {
    let watched: Duration = FIRST_UNIT_TIMEOUT
        + (SETTLE + MEASURED) * u32::try_from(phases).map_err(|e| e.to_string())?;
    let socket: Box<tungstenite::WebSocket<std::net::TcpStream>> =
        match viewer::handshake(emulator.port, Some(LOGIN))? {
            Handshake::Upgraded(socket) => socket,
            Handshake::Refused(status) => {
                return Err(format!("viewer handshake refused with HTTP {status}"));
            }
        };
    thread::spawn(move || viewer::watch(socket, VIEWER_VIEW, watched));
    println!(
        "INFO viewer acking each frame on receipt, view {}x{}, through the emulator on port {}",
        VIEWER_VIEW.0, VIEWER_VIEW.1, emulator.port
    );
    Ok(None)
}

fn streamed(
    report: &mut Report,
    instance: &Instance,
    scratch: &Scratch,
    plan: &Plan,
    phases: &[Profile],
) -> Result<Vec<Summary>, String> {
    let first: Profile = phases.first().cloned().ok_or("no profile to measure")?;
    let emulator: Emulator = Emulator::start(PORT, first)?;
    let mover: Mover = Mover::start()?;
    let browser: Option<Browser> = match plan {
        Plan::Profiles(_) => page(scratch, &emulator)?,
        Plan::Calibrate => viewer(&emulator, phases.len())?,
    };
    report.line(
        "first-unit",
        true,
        format!("first acked unit after {:?}", first_unit(instance.lines())?),
    );
    let summaries: Result<Vec<Summary>, String> = measure(report, instance, &emulator, phases);
    browser.into_iter().for_each(Browser::close);
    mover.stop()?;
    summaries
}

fn calibrated(report: &mut Report, summaries: &[Summary]) -> Result<(), String> {
    let [base, rtt_100, rtt_200, capped]: &[Summary; 4] = summaries
        .try_into()
        .map_err(|_| format!("calibration measured {} phases, not 4", summaries.len()))?;
    report.record(stats::added_rtt_check(100.0, base, rtt_100));
    report.record(stats::added_rtt_check(200.0, base, rtt_200));
    report.record(stats::cap_check(
        "stream-cap",
        capped.mbit_per_s,
        None,
        CAP_CEILING_MBIT,
    ));
    let bulk: f64 = proxy::throughput(link::parse(CAP)?, BULK_BYTES)?;
    report.record(stats::cap_check(
        "bulk-cap",
        bulk / 1e6,
        Some(CAP_FLOOR_MBIT),
        CAP_CEILING_MBIT,
    ));
    Ok(())
}

fn bench(report: &mut Report, bin: Option<PathBuf>, plan: Plan) -> Result<(), String> {
    let bin: PathBuf = match bin {
        Some(bin) => bin,
        None => testbed::build(&crate::views::own_root()?)?,
    };
    println!("INFO binary: {}", bin.display());
    drop(
        TcpListener::bind((Ipv4Addr::LOCALHOST, PORT))
            .map_err(|e| format!("port {PORT} is not free: {e}"))?,
    );
    let phases: Vec<Profile> = match &plan {
        Plan::Profiles(profiles) => profiles.clone(),
        Plan::Calibrate => calibration()?,
    };
    let scratch: Scratch = Scratch::create("latency")?;
    let runtime: Runtime = Runtime::create(RUNTIME_NAME)?;
    runtime
        .checks()
        .into_iter()
        .for_each(|check: Check| report.record(check));
    let path = scratch.path()?;
    report.record(testbed::stub_check(&scratch, &path)?);
    if !report.passed() {
        return Ok(());
    }
    let mut instance: Instance = Instance::spawn(&bin, &runtime, &path, (PORT, LOGIN), &[TIMINGS])?;
    let listened: Duration = instance.listen()?;
    report.line("listening", true, format!("listen line after {listened:?}"));
    let summaries: Result<Vec<Summary>, String> =
        streamed(report, &instance, &scratch, &plan, &phases);
    let stopped: Result<testbed::Shutdown, String> = instance.stop();
    instance
        .stderr()
        .iter()
        .for_each(|line: &String| println!("INFO phonepad stderr: {line}"));
    let summaries: Vec<Summary> = summaries?;
    println!("INFO test instance: {:?}", stopped?);
    println!("{}", stats::HEADER);
    phases
        .iter()
        .zip(&summaries)
        .for_each(|(profile, summary): (&Profile, &Summary)| {
            println!("{}", stats::row(profile, summary))
        });
    match plan {
        Plan::Profiles(_) => Ok(()),
        Plan::Calibrate => calibrated(report, &summaries),
    }
}

pub fn run(bin: Option<PathBuf>, plan: Plan) -> bool {
    let mut report: Report = Report::default();
    if let Err(error) = bench(&mut report, bin, plan) {
        report.line("latency", false, error);
    }
    report.passed()
}

#[cfg(test)]
mod tests;
