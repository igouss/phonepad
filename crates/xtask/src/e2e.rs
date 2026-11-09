mod client;
mod verdict;

use crate::report::{Check, Report};
use client::{Handshake, Received};
use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader};
use std::net::{Ipv4Addr, TcpListener};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle, ScopedJoinHandle};
use std::time::{Duration, Instant};
use verdict::{Decoded, Shutdown, Tally};

const PORT: u16 = 8769;
const LOGIN: &str = "test@example.invalid";
const WRONG_LOGIN: &str = "intruder@example.invalid";
const RUNTIME_NAME: &str = "ppe2e";
const VIEW: (u32, u32) = (648, 1072);
const WATCHED: Duration = Duration::from_secs(4);
const VIEWER_COUNTS: [usize; 2] = [1, 3];
const LISTEN_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(5);
const SHUTDOWN_PATIENCE: Duration = Duration::from_secs(20);
const EXIT_POLL: Duration = Duration::from_millis(5);
const STUB_OWE: &str = "#!/bin/sh\ncase \"$1\" in status) echo '{\"status\":\"ok\",\"paused\":false,\"manual_pause\":false}';; esac\nexit 0\n";

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn create() -> Result<Scratch, String> {
        let scratch: Scratch = Scratch {
            dir: std::env::temp_dir().join(format!("phonepad-e2e-{}", std::process::id())),
        };
        let owe: PathBuf = scratch.bin().join("owe");
        std::fs::create_dir_all(scratch.bin())
            .map_err(|e| format!("{}: {e}", scratch.dir.display()))?;
        std::fs::write(&owe, STUB_OWE).map_err(|e| format!("{}: {e}", owe.display()))?;
        std::fs::set_permissions(&owe, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("{}: {e}", owe.display()))?;
        Ok(scratch)
    }

    fn bin(&self) -> PathBuf {
        self.dir.join("bin")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.dir) {
            eprintln!("{}: {error}", self.dir.display());
        }
    }
}

struct Runtime {
    dir: PathBuf,
    signature: String,
    display: String,
}

fn live_instances(hypr: &Path) -> Result<Vec<String>, String> {
    let entries: std::fs::ReadDir =
        std::fs::read_dir(hypr).map_err(|e| format!("{}: {e}", hypr.display()))?;
    Ok(entries
        .filter_map(Result::ok)
        .filter(|entry: &std::fs::DirEntry| {
            UnixStream::connect(entry.path().join(".socket.sock")).is_ok()
        })
        .map(|entry: std::fs::DirEntry| entry.file_name().to_string_lossy().into_owned())
        .collect())
}

impl Runtime {
    fn create() -> Result<Runtime, String> {
        let parent: PathBuf = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .ok_or("XDG_RUNTIME_DIR is not set")?;
        let display: String =
            std::env::var("WAYLAND_DISPLAY").map_err(|e| format!("WAYLAND_DISPLAY: {e}"))?;
        let signature: String = verdict::instance(
            std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok().as_deref(),
            &live_instances(&parent.join("hypr"))?,
        )?;
        let dir: PathBuf = parent.join(RUNTIME_NAME);
        if std::fs::symlink_metadata(&dir).is_ok() {
            std::fs::remove_dir_all(&dir).map_err(|e| format!("stale {}: {e}", dir.display()))?;
        }
        std::fs::create_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let runtime: Runtime = Runtime {
            dir,
            signature,
            display,
        };
        for name in [OsStr::new("hypr"), OsStr::new(&runtime.display)] {
            let link: PathBuf = runtime.dir.join(name);
            symlink(parent.join(name), &link).map_err(|e| format!("{}: {e}", link.display()))?;
        }
        Ok(runtime)
    }

    fn socket_check(gate: &str, socket: &Path) -> Check {
        let connected: Result<UnixStream, std::io::Error> = UnixStream::connect(socket);
        Check {
            gate: gate.to_owned(),
            ok: connected.is_ok(),
            detail: format!(
                "{} ({} bytes): {}",
                socket.display(),
                socket.as_os_str().len(),
                connected.map_or_else(|e| e.to_string(), |_| String::from("connects"))
            ),
        }
    }

    fn checks(&self) -> [Check; 2] {
        [
            Runtime::socket_check(
                "hypr-socket",
                &self
                    .dir
                    .join("hypr")
                    .join(&self.signature)
                    .join(".socket.sock"),
            ),
            Runtime::socket_check("wayland-socket", &self.dir.join(&self.display)),
        ]
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.dir) {
            eprintln!("{}: {error}", self.dir.display());
        }
    }
}

struct Instance {
    child: Child,
    argv: [OsString; 3],
    stdout: Receiver<String>,
    stderr: Option<JoinHandle<Vec<String>>>,
}

fn forward_lines(stdout: ChildStdout, lines: Sender<String>) {
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if lines.send(line).is_err() {
            return;
        }
    }
}

fn collect_lines(stderr: ChildStderr) -> Vec<String> {
    BufReader::new(stderr)
        .lines()
        .map_while(Result::ok)
        .collect()
}

impl Instance {
    fn spawn(bin: &Path, runtime: &Runtime, path: &OsStr) -> Result<Instance, String> {
        let argv: [OsString; 3] = [
            bin.as_os_str().to_owned(),
            OsString::from(PORT.to_string()),
            OsString::from(LOGIN),
        ];
        let mut child: Child = Command::new(&argv[0])
            .args(&argv[1..])
            .env("XDG_RUNTIME_DIR", &runtime.dir)
            .env("WAYLAND_DISPLAY", &runtime.display)
            .env("HYPRLAND_INSTANCE_SIGNATURE", &runtime.signature)
            .env("PATH", path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn {}: {e}", bin.display()))?;
        let (lines, stdout): (Sender<String>, Receiver<String>) = mpsc::channel();
        let out: Option<ChildStdout> = child.stdout.take();
        let err: Option<ChildStderr> = child.stderr.take();
        let instance: Instance = Instance {
            child,
            argv,
            stdout,
            stderr: err.map(|err: ChildStderr| thread::spawn(move || collect_lines(err))),
        };
        let out: ChildStdout = out.ok_or("child has no stdout")?;
        thread::spawn(move || forward_lines(out, lines));
        Ok(instance)
    }

    fn listen(&self) -> Result<Duration, String> {
        let start: Instant = Instant::now();
        loop {
            match self
                .stdout
                .recv_timeout(LISTEN_TIMEOUT.saturating_sub(start.elapsed()))
            {
                Ok(line) if verdict::listening(&line, PORT) => return Ok(start.elapsed()),
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!("no listen line within {LISTEN_TIMEOUT:?}"));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(String::from("stdout closed before the listen line"));
                }
            }
        }
    }

    fn running(&mut self) -> Check {
        let state: Result<Option<ExitStatus>, std::io::Error> = self.child.try_wait();
        Check {
            gate: String::from("still-running"),
            ok: matches!(state, Ok(None)),
            detail: format!("after the viewer sessions: {state:?}"),
        }
    }

    fn owned(&self) -> Result<u32, String> {
        let pid: u32 = self.child.id();
        let cmdline: Vec<u8> = std::fs::read(format!("/proc/{pid}/cmdline"))
            .map_err(|e| format!("/proc/{pid}/cmdline: {e}"))?;
        let argv: Vec<&OsStr> = self.argv.iter().map(OsString::as_os_str).collect();
        if verdict::spawned(&cmdline, &argv) {
            Ok(pid)
        } else {
            Err(format!("PID {pid} is not the instance this run spawned"))
        }
    }

    fn terminate(&mut self) -> Result<Shutdown, String> {
        if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
            return Ok(Shutdown::Early {
                code: status.code(),
            });
        }
        let pid: u32 = self.owned()?;
        let start: Instant = Instant::now();
        let sent: ExitStatus = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .map_err(|e| format!("kill -TERM {pid}: {e}"))?;
        if !sent.success() {
            return Err(format!("kill -TERM {pid}: {sent}"));
        }
        while start.elapsed() < SHUTDOWN_PATIENCE {
            if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
                return Ok(Shutdown::Exited {
                    after: start.elapsed(),
                    code: status.code(),
                });
            }
            thread::sleep(EXIT_POLL);
        }
        let waited: Duration = start.elapsed();
        self.child.kill().map_err(|e| format!("kill {pid}: {e}"))?;
        self.child.wait().map_err(|e| format!("wait {pid}: {e}"))?;
        Ok(Shutdown::Hung { waited })
    }

    fn reap(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            if let Err(error) = self.child.kill() {
                eprintln!("kill {}: {error}", self.child.id());
            }
            if let Err(error) = self.child.wait() {
                eprintln!("wait {}: {error}", self.child.id());
            }
        }
    }

    fn stop(&mut self) -> Result<Shutdown, String> {
        let shutdown: Result<Shutdown, String> = self.terminate();
        self.reap();
        shutdown
    }

    fn stderr(&mut self) -> Vec<String> {
        self.stderr
            .take()
            .map_or_else(Vec::new, |reader: JoinHandle<Vec<String>>| {
                reader.join().unwrap_or_default()
            })
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        self.reap();
    }
}

fn build(root: &Path) -> Result<PathBuf, String> {
    let cargo: OsString = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let output: Output = Command::new(cargo)
        .current_dir(root)
        .args([
            "build",
            "-p",
            "phonepad",
            "--message-format=json-render-diagnostics",
        ])
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("cargo build: {e}"))?;
    if !output.status.success() {
        return Err(format!("cargo build -p phonepad: {}", output.status));
    }
    verdict::executable(&String::from_utf8_lossy(&output.stdout), "phonepad")
        .ok_or_else(|| String::from("cargo build named no phonepad executable"))
}

fn stub_check(scratch: &Scratch, path: &OsStr) -> Result<Check, String> {
    let found: Output = Command::new("sh")
        .args(["-c", "command -v owe"])
        .env("PATH", path)
        .output()
        .map_err(|e| format!("sh: {e}"))?;
    let resolved: String = String::from_utf8_lossy(&found.stdout).trim().to_owned();
    let stub: PathBuf = scratch.bin().join("owe");
    Ok(Check {
        gate: String::from("stub-owe"),
        ok: Path::new(&resolved) == stub,
        detail: format!("owe resolves to {resolved:?}, stub {}", stub.display()),
    })
}

fn refused(login: Option<&str>) -> Result<u16, String> {
    match client::handshake(PORT, login)? {
        Handshake::Upgraded(_) => Ok(101),
        Handshake::Refused(status) => Ok(status),
    }
}

fn watch() -> Result<Received, String> {
    match client::handshake(PORT, Some(LOGIN))? {
        Handshake::Upgraded(socket) => client::watch(socket, VIEW, WATCHED),
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
        let handles: Vec<ScopedJoinHandle<Result<Received, String>>> =
            (0..viewers).map(|_| scope.spawn(watch)).collect();
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
        None => build(&crate::views::own_root()?)?,
    };
    println!("INFO binary: {}", bin.display());
    drop(
        TcpListener::bind((Ipv4Addr::LOCALHOST, PORT))
            .map_err(|e| format!("port {PORT} is not free: {e}"))?,
    );
    let scratch: Scratch = Scratch::create()?;
    let runtime: Runtime = Runtime::create()?;
    runtime
        .checks()
        .into_iter()
        .for_each(|check: Check| report.record(check));
    let path: OsString = std::env::join_paths(std::iter::once(scratch.bin()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .map_err(|e| format!("PATH: {e}"))?;
    report.record(stub_check(&scratch, &path)?);
    if !report.passed() {
        return Ok(());
    }
    let mut instance: Instance = Instance::spawn(&bin, &runtime, &path)?;
    let served: Result<(), String> = match instance.listen() {
        Ok(after) => {
            report.line("listening", true, format!("listen line after {after:?}"));
            served(report, &scratch, &mut instance)
        }
        Err(error) => {
            report.line("listening", false, error);
            Ok(())
        }
    };
    let shutdown: Result<Shutdown, String> = instance.stop();
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
