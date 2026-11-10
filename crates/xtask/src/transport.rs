mod probe;
mod relay;
mod replay;

use crate::latency::link::{self, Profile};
use crate::latency::proxy::Emulator;
use crate::latency::stats::Spread;
use crate::report::{Check, Report};
use probe::{Echo, Echoed, Sink};
use relay::Relay;
use replay::{Bound, Congestion, Knobs, Outcome, Transport};
use std::time::Duration;

const SETTLE: Duration = Duration::from_secs(3);
const DEFAULT_SECONDS: u64 = 60;
const DEFAULT_PORT: u16 = 8841;
const DEFAULT_DELTAS: (u32, u32) = (200, 600);
const MAX_LOAD: f64 = 3.0;
const LOAD_WAIT: Duration = Duration::from_secs(15);
const LOAD_TRIES: u32 = 40;
const PROBES: usize = 400;
const PROBE_EVERY: Duration = Duration::from_millis(5);
const RTT_TOLERANCE_MS: f64 = 5.0;
const LOSS_DATAGRAMS: usize = 20_000;
const LOSS_EVERY: Duration = Duration::from_micros(100);
const RATE_DATAGRAMS: usize = 750;
const RATE_BYTES: usize = 1200;
const RATE_EVERY: Duration = Duration::from_micros(3200);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Path {
    Emulated,
    Direct,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arm {
    pub transport: Transport,
    pub path: Path,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    Calibrate {
        direct: bool,
    },
    Serve {
        port: u16,
        seconds: u64,
    },
    Compare {
        arms: Vec<Arm>,
        profile: Profile,
        seconds: u64,
        port: u16,
        rounds: u32,
        knobs: Knobs,
    },
}

fn arm(word: &str) -> Option<Arm> {
    let (transport, path): (Transport, Path) = match word {
        "ws-proxy" => (Transport::WebSocket, Path::Emulated),
        "wt-relay" => (Transport::WebTransport, Path::Emulated),
        "ws-direct" => (Transport::WebSocket, Path::Direct),
        "wt-direct" => (Transport::WebTransport, Path::Direct),
        _ => return None,
    };
    Some(Arm { transport, path })
}

fn arm_name(arm: Arm) -> &'static str {
    match (arm.transport, arm.path) {
        (Transport::WebSocket, Path::Emulated) => "ws-proxy",
        (Transport::WebTransport, Path::Emulated) => "wt-relay",
        (Transport::WebSocket, Path::Direct) => "ws-direct",
        (Transport::WebTransport, Path::Direct) => "wt-direct",
    }
}

fn value<T: std::str::FromStr>(word: &str, key: &str) -> Option<Result<T, String>> {
    word.strip_prefix(key).map(|text: &str| {
        text.parse::<T>()
            .map_err(|_| format!("{key}{text} is not a number"))
    })
}

pub fn plan(words: &[&str]) -> Result<Plan, String> {
    match words {
        ["calibrate"] => return Ok(Plan::Calibrate { direct: false }),
        ["calibrate", "direct"] => return Ok(Plan::Calibrate { direct: true }),
        ["serve-wt", seconds] => {
            return Ok(Plan::Serve {
                port: DEFAULT_PORT,
                seconds: value::<u64>(seconds, "seconds=")
                    .ok_or("serve-wt takes seconds=<s>")??,
            });
        }
        _ => {}
    }
    let mut arms: Vec<Arm> = Vec::new();
    let mut profile: Option<Profile> = None;
    let mut seconds: u64 = DEFAULT_SECONDS;
    let mut port: u16 = DEFAULT_PORT;
    let mut rounds: u32 = 1;
    let mut knobs: Knobs = Knobs {
        deltas: DEFAULT_DELTAS,
        congestion: Congestion::Cubic,
    };
    for word in words {
        if let Some(parsed) = value::<u64>(word, "seconds=") {
            seconds = parsed?;
        } else if let Some(parsed) = value::<u16>(word, "port=") {
            port = parsed?;
        } else if let Some(parsed) = value::<u32>(word, "rounds=") {
            rounds = parsed?;
        } else if let Some(range) = word.strip_prefix("deltas=") {
            knobs.deltas = deltas(range)?;
        } else if *word == "cc=bbr" {
            knobs.congestion = Congestion::Bbr;
        } else if *word == "cc=cubic" {
            knobs.congestion = Congestion::Cubic;
        } else if let Some(found) = arm(word) {
            arms.push(found);
        } else {
            profile = Some(link::parse(word)?);
        }
    }
    if arms.is_empty() {
        return Err(String::from(
            "name at least one arm: ws-proxy, wt-relay, ws-direct, wt-direct",
        ));
    }
    Ok(Plan::Compare {
        arms,
        profile: profile.map_or_else(|| link::parse("lan"), Ok)?,
        seconds,
        port,
        rounds,
        knobs,
    })
}

fn deltas(range: &str) -> Result<(u32, u32), String> {
    let (low, high): (&str, &str) = range
        .split_once('-')
        .ok_or_else(|| format!("deltas={range} is not <low>-<high> bytes"))?;
    let parse = |text: &str| {
        text.parse::<u32>()
            .map_err(|_| format!("deltas={range}: {text} is not a byte count"))
    };
    let (low, high): (u32, u32) = (parse(low)?, parse(high)?);
    if low < high {
        Ok((low, high))
    } else {
        Err(format!(
            "deltas={range}: the low bound must be below the high"
        ))
    }
}

fn load() -> Option<f64> {
    std::fs::read_to_string("/proc/loadavg")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn wait_for_low_load() -> Result<f64, String> {
    for _ in 0..LOAD_TRIES {
        match load() {
            Some(now) if now < MAX_LOAD => return Ok(now),
            Some(now) => println!("INFO loadavg {now:.2} >= {MAX_LOAD}, waiting"),
            None => return Err(String::from("cannot read /proc/loadavg")),
        }
        std::thread::sleep(LOAD_WAIT);
    }
    Err(format!("loadavg stayed at or above {MAX_LOAD}"))
}

fn cell(spread: Option<Spread>) -> String {
    spread.map_or_else(
        || String::from("-"),
        |s: Spread| format!("{:.0} / {:.0} / {:.0}", s.p50, s.p95, s.max),
    )
}

const HEADER: &str = "| arm | round | load | units/s | Mbit/s | stalls > 250 / > 500 ms | longest gap ms | send→draw ms p50 / p95 / max | send→ack ms | ping RTT ms | pings unanswered | keyframes | desyncs | emulator drops up / down |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|---|";

fn row(arm: Arm, round: u32, load: f64, outcome: &Outcome, drops: &str) -> String {
    format!(
        "| {} | {round} | {load:.2} | {:.1} | {:.2} | {} / {} | {} | {} | {} | {} | {} | {} | {} | {drops} |",
        arm_name(arm),
        outcome.units_per_s,
        outcome.mbit_per_s,
        outcome.stalls_250,
        outcome.stalls_500,
        outcome
            .longest_gap_ms
            .map_or_else(|| String::from("-"), |gap: f64| format!("{gap:.0}")),
        cell(outcome.send_draw),
        cell(outcome.send_ack),
        cell(outcome.ping),
        outcome.pings_unanswered,
        outcome.keyframes,
        outcome.desyncs,
    )
}

fn bind_port(transport: Transport, port: u16) -> u16 {
    match transport {
        Transport::WebSocket => port,
        Transport::WebTransport => 0,
    }
}

fn one(
    runtime: &tokio::runtime::Runtime,
    arm: Arm,
    profile: &Profile,
    (port, knobs): (u16, Knobs),
    measured: Duration,
) -> Result<(Outcome, String), String> {
    let (bound, server_port): (Bound, u16) = runtime.block_on(replay::bind(
        arm.transport,
        bind_port(arm.transport, port),
        knobs.congestion,
    ))?;
    match (arm.transport, arm.path) {
        (Transport::WebSocket, Path::Emulated) => {
            let emulator: Emulator = Emulator::start(server_port, profile.clone())?;
            let outcome: Outcome =
                runtime.block_on(replay::run(bound, emulator.port, knobs, (SETTLE, measured)))?;
            Ok((outcome, String::from("modelled (stall)")))
        }
        (Transport::WebTransport, Path::Emulated) => {
            let relay: Relay = Relay::start(server_port, profile.clone())?;
            let outcome: Outcome =
                runtime.block_on(replay::run(bound, relay.port, knobs, (SETTLE, measured)))?;
            let (up_carried, up_lost, up_overflow) = relay.up.snapshot();
            let (down_carried, down_lost, down_overflow) = relay.down.snapshot();
            Ok((
                outcome,
                format!(
                    "lost {up_lost} of {} / {down_lost} of {}, overflow {up_overflow} / {down_overflow}",
                    up_carried + up_lost + up_overflow,
                    down_carried + down_lost + down_overflow
                ),
            ))
        }
        (Transport::WebSocket | Transport::WebTransport, Path::Direct) => {
            let outcome: Outcome =
                runtime.block_on(replay::run(bound, server_port, knobs, (SETTLE, measured)))?;
            Ok((outcome, String::from("kernel (netem if set)")))
        }
    }
}

fn compare(
    report: &mut Report,
    arms: &[Arm],
    profile: &Profile,
    (seconds, port, rounds): (u64, u16, u32),
    knobs: Knobs,
) -> Result<(), String> {
    let runtime: tokio::runtime::Runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    println!(
        "INFO profile {} ({} ms ±{} ms, {:?} bit/s, {} ppm loss); {} s measured after {} s settle; units every {:?}, deltas {}–{} B, keyframes {} B, QUIC congestion {:?}",
        profile.name,
        profile.rtt.as_millis(),
        profile.jitter.as_millis(),
        profile.rate,
        profile.loss_ppm,
        seconds,
        SETTLE.as_secs(),
        replay::TICK,
        knobs.deltas.0,
        knobs.deltas.1,
        replay::KEYFRAME_BYTES,
        knobs.congestion
    );
    let mut rows: Vec<String> = Vec::new();
    for round in 1..=rounds {
        for arm in arms {
            let load: f64 = wait_for_low_load()?;
            let (outcome, drops): (Outcome, String) = one(
                &runtime,
                *arm,
                profile,
                (port, knobs),
                Duration::from_secs(seconds),
            )?;
            let line: String = row(*arm, round, load, &outcome, &drops);
            println!("{line}");
            report.line(
                &format!("{} round {round}", arm_name(*arm)),
                outcome.units_per_s > 0.0,
                format!("{:.1} units/s", outcome.units_per_s),
            );
            rows.push(line);
        }
    }
    println!("{HEADER}");
    rows.iter().for_each(|line: &String| println!("{line}"));
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

fn rtt_check(added_ms: f64, base: &Echoed, with: &Echoed) -> Check {
    let medians: Option<(f64, f64)> = base
        .rtt
        .zip(with.rtt)
        .map(|(b, w): (Spread, Spread)| (b.p50, w.p50));
    Check {
        gate: format!("udp-added-rtt-{added_ms:.0}"),
        ok: medians.is_some_and(|(b, w): (f64, f64)| (w - b - added_ms).abs() <= RTT_TOLERANCE_MS),
        detail: medians.map_or_else(
            || String::from("no probe came back"),
            |(b, w): (f64, f64)| {
                format!(
                    "probe RTT p50 {w:.2} ms vs {b:.2} ms: +{:.2} ms, expected +{added_ms:.0} ± {RTT_TOLERANCE_MS:.0}",
                    w - b
                )
            },
        ),
    }
}

fn jitter_check(rtt_ms: f64, jitter_ms: f64, echoed: &Echoed) -> Check {
    let (low, high): (f64, f64) = (rtt_ms - jitter_ms, rtt_ms + jitter_ms + RTT_TOLERANCE_MS);
    let bounds: Option<(f64, f64, f64)> = echoed
        .min
        .zip(echoed.rtt)
        .zip(echoed.p5)
        .map(|((min, spread), p5): ((f64, Spread), f64)| (min, p5, spread.max));
    Check {
        gate: format!("udp-jitter-{jitter_ms:.0}"),
        ok: bounds.is_some_and(|(min, p5, max): (f64, f64, f64)| {
            min >= low && max <= high && max - p5 >= jitter_ms / 2.0
        }),
        detail: bounds.map_or_else(
            || String::from("no probe came back"),
            |(min, p5, max): (f64, f64, f64)| {
                format!(
                    "RTT min {min:.1} p5 {p5:.1} max {max:.1} ms, expected within [{low:.0}, {high:.0}] and max − p5 ≥ {:.0}",
                    jitter_ms / 2.0
                )
            },
        ),
    }
}

fn loss_check(loss: f64, sent: usize, received: usize) -> Check {
    let measured: f64 = 1.0 - received as f64 / sent as f64;
    let sigma: f64 = (loss * (1.0 - loss) / sent as f64).sqrt();
    Check {
        gate: format!("udp-loss-{:.0}%", loss * 100.0),
        ok: (measured - loss).abs() <= 3.0 * sigma,
        detail: format!(
            "{received} of {sent} datagrams reached the sink: {:.3}% lost, expected {:.1}% ± {:.3}% (3σ)",
            measured * 100.0,
            loss * 100.0,
            3.0 * sigma * 100.0
        ),
    }
}

fn echo_through(relay: Option<&Relay>, echo: &Echo) -> Result<Echoed, String> {
    probe::echo(
        relay.map_or(echo.port, |relay: &Relay| relay.port),
        PROBES,
        PROBE_EVERY,
    )
}

fn describe(name: &str, echoed: &Echoed) {
    println!(
        "INFO {name}: {} of {} probes back, RTT p50 / p95 / max {}, p5 {:.2}, min {:.2}",
        echoed.returned,
        echoed.sent,
        cell(echoed.rtt),
        echoed.p5.unwrap_or(f64::NAN),
        echoed.min.unwrap_or(f64::NAN)
    );
}

fn calibrate(report: &mut Report, direct: bool) -> Result<(), String> {
    let echo: Echo = Echo::start()?;
    if direct {
        let echoed: Echoed = echo_through(None, &echo)?;
        describe("direct probe", &echoed);
        let sink: Sink = Sink::start()?;
        probe::blast(sink.port, LOSS_DATAGRAMS, 100, LOSS_EVERY)?;
        let (received, _): (usize, f64) = sink.finish()?;
        println!(
            "INFO direct one-way: {received} of {LOSS_DATAGRAMS} datagrams reached the sink ({:.3}% lost)",
            (1.0 - received as f64 / LOSS_DATAGRAMS as f64) * 100.0
        );
        let sink: Sink = Sink::start()?;
        probe::blast(sink.port, RATE_DATAGRAMS, RATE_BYTES, RATE_EVERY)?;
        let (_, mbit): (usize, f64) = sink.finish()?;
        println!("INFO direct paced 3 Mbit/s send measured {mbit:.3} Mbit/s at the sink");
        return Ok(());
    }
    let relay: Relay = Relay::start(echo.port, link::parse("rtt=0")?)?;
    let base: Echoed = echo_through(Some(&relay), &echo)?;
    describe("rtt=0", &base);
    for added in [100.0, 200.0] {
        relay.set(link::parse(&format!("rtt={added}"))?);
        let with: Echoed = echo_through(Some(&relay), &echo)?;
        describe(&format!("rtt={added}"), &with);
        report.record(rtt_check(added, &base, &with));
    }
    relay.set(link::parse("rtt=150,jitter=50")?);
    let jittered: Echoed = echo_through(Some(&relay), &echo)?;
    describe("rtt=150,jitter=50", &jittered);
    report.record(jitter_check(150.0, 50.0, &jittered));
    drop(relay);
    let sink: Sink = Sink::start()?;
    let lossy: Relay = Relay::start(sink.port, link::parse("loss=1")?)?;
    probe::blast(lossy.port, LOSS_DATAGRAMS, 100, LOSS_EVERY)?;
    let (received, _): (usize, f64) = sink.finish()?;
    let (carried, lost, overflow) = lossy.up.snapshot();
    println!("INFO relay counted {carried} carried, {lost} lost, {overflow} overflowed");
    report.record(loss_check(0.01, LOSS_DATAGRAMS, received));
    drop(lossy);
    let sink: Sink = Sink::start()?;
    let capped: Relay = Relay::start(sink.port, link::parse("rate=2")?)?;
    probe::blast(capped.port, RATE_DATAGRAMS, RATE_BYTES, RATE_EVERY)?;
    let (_, mbit): (usize, f64) = sink.finish()?;
    let (_, _, overflow) = capped.up.snapshot();
    report.record(Check {
        gate: String::from("udp-cap-2"),
        ok: (1.9..=2.1).contains(&mbit) && overflow > 0,
        detail: format!(
            "a 3 Mbit/s paced send through the 2 Mbit/s cap measured {mbit:.3} Mbit/s at the sink, {overflow} datagrams overflowed the {}-datagram queue",
            relay::QUEUE
        ),
    });
    Ok(())
}

const SMOKE_SCRIPT: &str = include_str!("transport/smoke.js");

fn smoke_page(port: u16, hash: &wtransport::tls::Sha256Digest) -> String {
    let bytes: Vec<String> = hash.as_ref().iter().map(u8::to_string).collect();
    format!(
        "<!doctype html><meta charset=utf-8><title>wt smoke</title><script>const TARGET = \"https://127.0.0.1:{port}/\"; const HASH = [{}];\n{SMOKE_SCRIPT}</script>",
        bytes.join(",")
    )
}

async fn serve_page(listener: tokio::net::TcpListener, page: String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    while let Ok((mut tcp, _)) = listener.accept().await {
        let mut request: [u8; 4096] = [0; 4096];
        if tcp.read(&mut request).await.is_err() {
            continue;
        }
        let reply: String = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}",
            page.len()
        );
        if tcp.write_all(reply.as_bytes()).await.is_err() {
            continue;
        }
    }
}

fn serve(report: &mut Report, port: u16, seconds: u64) -> Result<(), String> {
    let runtime: tokio::runtime::Runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let server: replay::WtServer = replay::wt_bind(port, Congestion::Cubic)?;
    let page: String = smoke_page(server.port, &server.hash);
    let listener: tokio::net::TcpListener = runtime
        .block_on(tokio::net::TcpListener::bind(("127.0.0.1", port + 1)))
        .map_err(|e| format!("page bind {}: {e}", port + 1))?;
    println!(
        "INFO WebTransport on https://127.0.0.1:{}/ (certificate sha-256 {}), smoke page on http://127.0.0.1:{}/",
        server.port,
        server.hash,
        port + 1
    );
    runtime.spawn(serve_page(listener, page));
    let log: replay::Log = runtime.block_on(replay::serve(
        server,
        DEFAULT_DELTAS,
        Duration::from_secs(seconds),
    ))?;
    report.line(
        "wt-page-acks",
        !log.acked.is_empty(),
        format!(
            "the page acked {} units in order ({} keyframes, {} desyncs)",
            log.acked.len(),
            log.keyframes,
            log.desyncs
        ),
    );
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

pub fn run(plan: Plan) -> bool {
    let mut report: Report = Report::default();
    let result: Result<(), String> = match &plan {
        Plan::Calibrate { direct } => calibrate(&mut report, *direct),
        Plan::Serve { port, seconds } => serve(&mut report, *port, *seconds),
        Plan::Compare {
            arms,
            profile,
            seconds,
            port,
            rounds,
            knobs,
        } => compare(
            &mut report,
            arms,
            profile,
            (*seconds, *port, *rounds),
            *knobs,
        ),
    };
    if let Err(error) = result {
        report.line("transport", false, error);
    }
    report.passed()
}
