mod args;
mod browser;
mod content;
mod desk;
pub mod link;
mod mover;
mod nested;
pub mod proxy;
mod seen;
pub mod stats;
mod touch;

use crate::report::{Check, Report};
use crate::testbed::{self, Compositor, Instance, Runtime, Scratch};
use crate::viewer::{self, Handshake, Received, Reports};
use args::{DEFAULT_PORT, Desktop};
pub use args::{Plan, Run, plan};
use browser::{Browser, PHONE};
use content::Painted;
use desk::Desk;
use link::Profile;
use mover::Mover;
use proxy::Emulator;
use seen::Rates;
use stats::{Sample, Summary};
use std::net::{Ipv4Addr, TcpListener};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use touch::Driver;

const LOGIN: &str = "test@example.invalid";
const RUNTIME_NAME: &str = "pplat";
const TIMINGS: (&str, &str) = ("PHONEPAD_TIMINGS", "1");
const SETTLE: Duration = Duration::from_secs(3);
const FIRST_UNIT_TIMEOUT: Duration = Duration::from_secs(20);
const VIEWER_VIEW: (u32, u32) = (648, 1072);
const ADDED_RTT_MS: [u64; 2] = [100, 200];
const CAP: &str = "rate=2";
const CAP_CEILING_MBIT: f64 = 2.1;
const CAP_FLOOR_MBIT: f64 = 1.9;
const BULK_BYTES: usize = 750_000;
const CONTENT_RATES: [u32; 3] = [0, 20, 125];

#[derive(Clone, Debug, PartialEq, Eq)]
struct Phase {
    profile: Profile,
    hz: Option<u32>,
}

fn runtime_name(port: u16) -> String {
    if port == DEFAULT_PORT {
        String::from(RUNTIME_NAME)
    } else {
        format!("{RUNTIME_NAME}{port}")
    }
}

fn calibration() -> Result<Vec<Profile>, String> {
    std::iter::once(String::from("rtt=0"))
        .chain(ADDED_RTT_MS.map(|ms: u64| format!("rtt={ms}")))
        .chain(std::iter::once(String::from(CAP)))
        .map(|text: String| link::parse(&text))
        .collect()
}

fn phases(plan: &Plan) -> Result<Vec<Phase>, String> {
    let hz: Option<u32> = match plan.desktop {
        Desktop::Live => None,
        Desktop::Nested { hz, .. } => Some(hz),
    };
    let phase = |profile: Profile| Phase { profile, hz };
    match &plan.run {
        Run::Page(profiles) | Run::Viewer(profiles) | Run::Touch(profiles) => {
            Ok(profiles.iter().cloned().map(phase).collect())
        }
        Run::Calibrate => Ok(calibration()?.into_iter().map(phase).collect()),
        Run::Content => {
            let lan: Profile = link::parse("lan")?;
            Ok(CONTENT_RATES
                .iter()
                .map(|hz: &u32| Phase {
                    profile: lan.clone(),
                    hz: Some(*hz),
                })
                .collect())
        }
    }
}

struct Collected {
    samples: Vec<Sample>,
    malformed: Vec<String>,
}

pub struct Measured {
    summary: Summary,
    acked: Vec<u32>,
    painted: Option<(Painted, Painted)>,
}

enum Client {
    Page(Box<Browser>),
    Viewer(JoinHandle<Result<Received, String>>),
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

fn acked(samples: &[Sample]) -> Vec<u32> {
    samples
        .iter()
        .filter_map(|sample: &Sample| match sample {
            Sample::Unit { id, .. } => Some(*id),
            Sample::Drawn { .. } => None,
        })
        .collect()
}

fn measure(
    report: &mut Report,
    instance: &Instance,
    emulator: &Emulator,
    (phases, desk): (&[Phase], Option<&Desk>),
    measured: Duration,
) -> Result<Vec<Measured>, String> {
    phases
        .iter()
        .map(|phase: &Phase| {
            emulator.set(phase.profile.clone());
            if let (Some(desk), Some(hz)) = (desk, phase.hz) {
                desk.content.set(hz);
            }
            collect(instance.lines(), SETTLE)?;
            let before: Option<Painted> = desk.map(|desk: &Desk| desk.content.painted());
            let collected: Collected = collect(instance.lines(), measured)?;
            let after: Option<Painted> = desk.map(|desk: &Desk| desk.content.painted());
            report.line(
                &format!("{} records", phase.profile.name),
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
            Ok(Measured {
                summary: stats::summarize(&collected.samples, measured),
                acked: acked(&collected.samples),
                painted: before.zip(after),
            })
        })
        .collect()
}

fn page(scratch: &Scratch, emulator: &Emulator, probe: bool) -> Result<Client, String> {
    let mut browser: Browser = Browser::launch(&scratch.dir.join("chromium"))?;
    if probe {
        browser.preload(touch::PROBE)?;
    }
    browser.open(&format!("http://127.0.0.1:{}/", emulator.port), LOGIN)?;
    println!(
        "INFO page {}x{} CSS px at {}x through the emulator on port {}: {}",
        PHONE.width,
        PHONE.height,
        PHONE.scale,
        emulator.port,
        browser.view()?
    );
    Ok(Client::Page(Box::new(browser)))
}

fn viewer(
    emulator: &Emulator,
    phases: usize,
    measured: Duration,
    reports: Reports,
) -> Result<Client, String> {
    let watched: Duration = FIRST_UNIT_TIMEOUT
        + (SETTLE + measured) * u32::try_from(phases).map_err(|e| e.to_string())?;
    let socket: Box<tungstenite::WebSocket<std::net::TcpStream>> =
        match viewer::handshake(emulator.port, Some(LOGIN))? {
            Handshake::Upgraded(socket) => socket,
            Handshake::Refused(status) => {
                return Err(format!("viewer handshake refused with HTTP {status}"));
            }
        };
    println!(
        "INFO viewer acking each frame on receipt, view {}x{}, through the emulator on port {}",
        VIEWER_VIEW.0, VIEWER_VIEW.1, emulator.port
    );
    Ok(Client::Viewer(thread::spawn(move || {
        viewer::watch(socket, VIEWER_VIEW, watched, reports)
    })))
}

fn streamed(
    report: &mut Report,
    instance: &Instance,
    (scratch, plan): (&Scratch, &Plan),
    phases: &[Phase],
    desk: Option<&Desk>,
) -> Result<(Vec<Measured>, Option<Received>), String> {
    let first: Profile = phases
        .first()
        .map(|phase: &Phase| phase.profile.clone())
        .ok_or("no profile to measure")?;
    let emulator: Emulator = Emulator::start(plan.port, first)?;
    let touching: bool = matches!(plan.run, Run::Touch(_));
    let mover: Option<Mover> = match (desk, touching) {
        (None, false) => Some(Mover::start()?),
        (Some(_), _) | (None, true) => None,
    };
    let client: Client = match plan.run {
        Run::Page(_) | Run::Touch(_) => page(scratch, &emulator, touching)?,
        Run::Viewer(_) => viewer(
            &emulator,
            phases.len(),
            plan.measured,
            Reports::AcksAndDrawn,
        )?,
        Run::Calibrate | Run::Content => {
            viewer(&emulator, phases.len(), plan.measured, Reports::Acks)?
        }
    };
    report.line(
        "first-unit",
        true,
        format!("first acked unit after {:?}", first_unit(instance.lines())?),
    );
    let (client, driver): (Option<Client>, Option<Driver>) = match (touching, client) {
        (true, Client::Page(browser)) => (
            None,
            Some(Driver::start(*browser, phases.len(), SETTLE, plan.measured)),
        ),
        (_, client) => (Some(client), None),
    };
    let measured: Result<Vec<Measured>, String> =
        measure(report, instance, &emulator, (phases, desk), plan.measured);
    let received: Option<Received> = match client {
        None => None,
        Some(Client::Page(browser)) => {
            browser.close();
            None
        }
        Some(Client::Viewer(watching)) => Some(
            watching
                .join()
                .map_err(|_| String::from("the viewer thread panicked"))??,
        ),
    };
    mover.map(Mover::stop).transpose()?;
    if let Some(driver) = driver {
        let names: Vec<&str> = phases
            .iter()
            .map(|phase: &Phase| phase.profile.name.as_str())
            .collect();
        touch::verdicts(&names, &driver.finish()?)
            .into_iter()
            .for_each(|check: Check| report.record(check));
    }
    Ok((measured?, received))
}

fn calibrated(report: &mut Report, summaries: &[&Summary]) -> Result<(), String> {
    let [base, rtt_100, rtt_200, capped]: &[&Summary; 4] = summaries
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

fn tabled(phases: &[Phase], measured: &[Measured]) {
    println!("{}", stats::HEADER);
    phases
        .iter()
        .zip(measured)
        .for_each(|(phase, measured): (&Phase, &Measured)| {
            println!("{}", stats::row(&phase.profile, &measured.summary))
        });
}

fn seen_table(report: &mut Report, plan: &Plan, phases: &[Phase], rates: &[Rates], desk: &Desk) {
    println!("{}", seen::HEADER);
    phases
        .iter()
        .zip(rates)
        .for_each(|(phase, rates): (&Phase, &Rates)| {
            println!("{}", seen::row(&phase.profile.name, rates))
        });
    if plan.run == Run::Content {
        rates
            .iter()
            .for_each(|rates: &Rates| report.record(seen::check(rates, desk.output.refresh_hz)));
    }
}

fn bench(report: &mut Report, bin: Option<PathBuf>, plan: Plan) -> Result<(), String> {
    let bin: PathBuf = match bin {
        Some(bin) => bin,
        None => testbed::build(&crate::views::own_root()?)?,
    };
    println!("INFO binary: {}", bin.display());
    let port: u16 = plan.port;
    drop(
        TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .map_err(|e| format!("port {port} is not free: {e}"))?,
    );
    let phases: Vec<Phase> = phases(&plan)?;
    let scratch: Scratch = Scratch::create("latency")?;
    let desk: Option<Desk> = match &plan.desktop {
        Desktop::Live => None,
        Desktop::Nested { hyprland, .. } => Some(Desk::start(report, hyprland, &scratch)?),
    };
    let compositor: Compositor = desk
        .as_ref()
        .map_or_else(Compositor::live, |desk: &Desk| Ok(desk.compositor()))?;
    let runtime: Runtime = Runtime::create(&runtime_name(port), &compositor)?;
    runtime
        .checks()
        .into_iter()
        .for_each(|check: Check| report.record(check));
    let path = scratch.path()?;
    report.record(testbed::stub_check(&scratch, &path)?);
    if !report.passed() {
        return Ok(());
    }
    let mut instance: Instance = Instance::spawn(&bin, &runtime, &path, (port, LOGIN), &[TIMINGS])?;
    let listened: Duration = instance.listen()?;
    report.line("listening", true, format!("listen line after {listened:?}"));
    let streamed: Result<(Vec<Measured>, Option<Received>), String> =
        streamed(report, &instance, (&scratch, &plan), &phases, desk.as_ref());
    let stopped: Result<testbed::Shutdown, String> = instance.stop();
    instance
        .stderr()
        .iter()
        .for_each(|line: &String| println!("INFO phonepad stderr: {line}"));
    let (measured, received): (Vec<Measured>, Option<Received>) = streamed?;
    println!("INFO test instance: {:?}", stopped?);
    tabled(&phases, &measured);
    if let Some(desk) = desk {
        if let Some(received) = received {
            let rates: Vec<Rates> =
                desk.rates(&scratch, &received, (&phases, &measured), plan.measured)?;
            seen_table(report, &plan, &phases, &rates, &desk);
        }
        desk.stop(report)?;
    }
    match plan.run {
        Run::Page(_) | Run::Viewer(_) | Run::Touch(_) | Run::Content => Ok(()),
        Run::Calibrate => calibrated(
            report,
            &measured
                .iter()
                .map(|measured: &Measured| &measured.summary)
                .collect::<Vec<&Summary>>(),
        ),
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
