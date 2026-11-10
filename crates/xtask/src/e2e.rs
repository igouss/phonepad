mod client;
mod verdict;

use crate::report::{Check, Report};
use crate::testbed::{self, Instance, Runtime, SHUTDOWN_PATIENCE, Scratch, Shutdown};
use client::{Handshake, Received};
use std::ffi::OsString;
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread::{self, JoinHandle, ScopedJoinHandle};
use std::time::Duration;
use verdict::{Decoded, Tally};

const PORT: u16 = 8769;
const LOGIN: &str = "test@example.invalid";
const WRONG_LOGIN: &str = "intruder@example.invalid";
const RUNTIME_NAME: &str = "ppe2e";
const VIEW: (u32, u32) = (648, 1072);
const WATCHED: Duration = Duration::from_secs(4);
const VIEWER_COUNTS: [usize; 2] = [1, 3];
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(5);
const STREAMING_BEFORE_SIGTERM: Duration = Duration::from_secs(1);

type Viewing = JoinHandle<Result<Received, String>>;

fn refused(login: Option<&str>) -> Result<u16, String> {
    match client::handshake(PORT, login)? {
        Handshake::Upgraded(_) => Ok(101),
        Handshake::Refused(status) => Ok(status),
    }
}

fn watch(watched: Duration) -> Result<Received, String> {
    match client::handshake(PORT, Some(LOGIN))? {
        Handshake::Upgraded(socket) => client::watch(socket, VIEW, watched),
        Handshake::Refused(status) => Err(format!("handshake refused with HTTP {status}")),
    }
}

fn decode(stream: &Path) -> Result<Decoded, String> {
    let output: Output = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-f", "h264", "-i"])
        .arg(stream)
        .args(["-f", "framemd5", "-"])
        .output()
        .map_err(|e| format!("cannot run ffmpeg: {e}"))?;
    let stdout: String = String::from_utf8_lossy(&output.stdout).into_owned();
    let hashes: Vec<&str> = verdict::frame_hashes(&stdout);
    let stderr: String = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Ok(Decoded {
        frames: hashes.len(),
        distinct: verdict::distinct(&hashes),
        errors: if output.status.success() {
            stderr
        } else {
            format!("ffmpeg {}: {stderr}", output.status)
        },
    })
}

fn session(report: &mut Report, scratch: &Scratch, viewers: usize) -> Result<(), String> {
    let results: Vec<Result<Received, String>> = thread::scope(|scope| {
        let handles: Vec<ScopedJoinHandle<Result<Received, String>>> = (0..viewers)
            .map(|_| scope.spawn(|| watch(WATCHED)))
            .collect();
        handles
            .into_iter()
            .map(|handle: ScopedJoinHandle<Result<Received, String>>| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err(String::from("viewer thread panicked")))
            })
            .collect()
    });
    for (index, result) in results.into_iter().enumerate() {
        let label: String = format!("{viewers}-viewers/{index}");
        let received: Received = match result {
            Ok(received) => received,
            Err(error) => {
                report.line(&label, false, error);
                continue;
            }
        };
        let tally: Tally = verdict::tally(&received.binary, received.texts);
        let stream: PathBuf = scratch
            .dir
            .join(format!("viewers{viewers}-client{index}.h264"));
        std::fs::write(&stream, &tally.stream).map_err(|e| format!("{}: {e}", stream.display()))?;
        let decoded: Decoded = decode(&stream)?;
        verdict::viewer_checks(&label, &tally, &decoded, WATCHED)
            .into_iter()
            .for_each(|check: Check| report.record(check));
    }
    Ok(())
}

fn served(report: &mut Report, scratch: &Scratch, instance: &mut Instance) -> Result<(), String> {
    report.record(verdict::status_check(
        "forbidden-wrong-login",
        refused(Some(WRONG_LOGIN)),
    ));
    report.record(verdict::status_check("forbidden-no-login", refused(None)));
    for viewers in VIEWER_COUNTS {
        session(report, scratch, viewers)?;
    }
    report.record(instance.running());
    Ok(())
}

fn checks(report: &mut Report, bin: Option<PathBuf>) -> Result<(), String> {
    let bin: PathBuf = match bin {
        Some(bin) => bin,
        None => testbed::build(&crate::views::own_root()?)?,
    };
    println!("INFO binary: {}", bin.display());
    drop(
        TcpListener::bind((Ipv4Addr::LOCALHOST, PORT))
            .map_err(|e| format!("port {PORT} is not free: {e}"))?,
    );
    let scratch: Scratch = Scratch::create("e2e")?;
    let runtime: Runtime = Runtime::create(RUNTIME_NAME)?;
    runtime
        .checks()
        .into_iter()
        .for_each(|check: Check| report.record(check));
    let path: OsString = scratch.path()?;
    report.record(testbed::stub_check(&scratch, &path)?);
    if !report.passed() {
        return Ok(());
    }
    let mut instance: Instance = Instance::spawn(&bin, &runtime, &path, (PORT, LOGIN))?;
    let (served, streaming): (Result<(), String>, Option<Viewing>) = match instance.listen() {
        Ok(after) => {
            report.line("listening", true, format!("listen line after {after:?}"));
            let served: Result<(), String> = served(report, &scratch, &mut instance);
            let streaming: Viewing = thread::spawn(|| watch(SHUTDOWN_PATIENCE + WATCHED));
            thread::sleep(STREAMING_BEFORE_SIGTERM);
            (served, Some(streaming))
        }
        Err(error) => {
            report.line("listening", false, error);
            (Ok(()), None)
        }
    };
    let shutdown: Result<Shutdown, String> = instance.stop();
    if let Some(streaming) = streaming {
        let outcome: String = match streaming.join() {
            Ok(Ok(received)) => {
                format!("{} messages, then the watch ran out", received.binary.len())
            }
            Ok(Err(error)) => error,
            Err(_) => String::from("viewer thread panicked"),
        };
        println!("INFO viewer streaming during SIGTERM: {outcome}");
    }
    instance
        .stderr()
        .iter()
        .for_each(|line: &String| println!("INFO phonepad stderr: {line}"));
    report.record(verdict::shutdown_check(&shutdown?, SHUTDOWN_LIMIT));
    served
}

pub fn run(bin: Option<PathBuf>) -> bool {
    let mut report: Report = Report::default();
    if let Err(error) = checks(&mut report, bin) {
        report.line("e2e", false, error);
    }
    report.passed()
}
