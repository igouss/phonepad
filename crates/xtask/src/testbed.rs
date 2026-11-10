use crate::report::Check;
use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const SHUTDOWN_PATIENCE: Duration = Duration::from_secs(20);
const LISTEN_TIMEOUT: Duration = Duration::from_secs(30);
const EXIT_POLL: Duration = Duration::from_millis(5);
const STUB_OWE: &str = "#!/bin/sh\ncase \"$1\" in status) echo '{\"status\":\"ok\",\"paused\":false,\"manual_pause\":false}';; esac\nexit 0\n";

#[derive(Debug, PartialEq, Eq)]
pub enum Shutdown {
    Exited { after: Duration, code: Option<i32> },
    Hung { waited: Duration },
    Early { code: Option<i32> },
}

pub struct Scratch {
    pub dir: PathBuf,
}

impl Scratch {
    pub fn create(name: &str) -> Result<Scratch, String> {
        let scratch: Scratch = Scratch {
            dir: std::env::temp_dir().join(format!(
                "phonepad-{name}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_nanos()
            )),
        };
        let owe: PathBuf = scratch.bin().join("owe");
        std::fs::create_dir_all(scratch.bin())
            .map_err(|e| format!("{}: {e}", scratch.dir.display()))?;
        std::fs::write(&owe, STUB_OWE).map_err(|e| format!("{}: {e}", owe.display()))?;
        std::fs::set_permissions(&owe, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("{}: {e}", owe.display()))?;
        Ok(scratch)
    }

    pub fn bin(&self) -> PathBuf {
        self.dir.join("bin")
    }

    pub fn path(&self) -> Result<OsString, String> {
        std::env::join_paths(std::iter::once(self.bin()).chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        )))
        .map_err(|e| format!("PATH: {e}"))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.dir) {
            eprintln!("{}: {error}", self.dir.display());
        }
    }
}

pub struct Runtime {
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
    pub fn create(name: &str) -> Result<Runtime, String> {
        let parent: PathBuf = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .ok_or("XDG_RUNTIME_DIR is not set")?;
        let display: String =
            std::env::var("WAYLAND_DISPLAY").map_err(|e| format!("WAYLAND_DISPLAY: {e}"))?;
        let signature: String = instance(
            std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok().as_deref(),
            &live_instances(&parent.join("hypr"))?,
        )?;
        let dir: PathBuf = parent.join(name);
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

    pub fn checks(&self) -> [Check; 2] {
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

pub struct Instance {
    child: Child,
    port: u16,
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
    pub fn spawn(
        bin: &Path,
        runtime: &Runtime,
        path: &OsStr,
        (port, login): (u16, &str),
        env: &[(&str, &str)],
    ) -> Result<Instance, String> {
        let argv: [OsString; 3] = [
            bin.as_os_str().to_owned(),
            OsString::from(port.to_string()),
            OsString::from(login),
        ];
        let mut child: Child = Command::new(&argv[0])
            .args(&argv[1..])
            .env("XDG_RUNTIME_DIR", &runtime.dir)
            .env("WAYLAND_DISPLAY", &runtime.display)
            .env("HYPRLAND_INSTANCE_SIGNATURE", &runtime.signature)
            .env("PATH", path)
            .envs(env.iter().copied())
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
            port,
            argv,
            stdout,
            stderr: err.map(|err: ChildStderr| thread::spawn(move || collect_lines(err))),
        };
        let out: ChildStdout = out.ok_or("child has no stdout")?;
        thread::spawn(move || forward_lines(out, lines));
        Ok(instance)
    }

    pub fn listen(&self) -> Result<Duration, String> {
        let start: Instant = Instant::now();
        loop {
            match self
                .stdout
                .recv_timeout(LISTEN_TIMEOUT.saturating_sub(start.elapsed()))
            {
                Ok(line) if listening(&line, self.port) => return Ok(start.elapsed()),
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

    pub fn lines(&self) -> &Receiver<String> {
        &self.stdout
    }

    pub fn running(&mut self) -> Check {
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
        if spawned(&cmdline, &argv) {
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

    pub fn stop(&mut self) -> Result<Shutdown, String> {
        let shutdown: Result<Shutdown, String> = self.terminate();
        self.reap();
        shutdown
    }

    pub fn stderr(&mut self) -> Vec<String> {
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

pub fn build(root: &Path) -> Result<PathBuf, String> {
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
    executable(&String::from_utf8_lossy(&output.stdout), "phonepad")
        .ok_or_else(|| String::from("cargo build named no phonepad executable"))
}

pub fn stub_check(scratch: &Scratch, path: &OsStr) -> Result<Check, String> {
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

pub fn listening(line: &str, port: u16) -> bool {
    line == format!("phonepad listening on http://127.0.0.1:{port}")
}

pub fn instance(inherited: Option<&str>, live: &[String]) -> Result<String, String> {
    match (inherited, live) {
        (Some(own), _) if live.iter().any(|sig: &String| sig == own) => Ok(own.to_owned()),
        (_, [only]) => Ok(only.clone()),
        (_, []) => Err(String::from(
            "no Hyprland instance socket accepts a connection",
        )),
        (_, [_, _, ..]) => Err(format!(
            "several live Hyprland instances {live:?} and HYPRLAND_INSTANCE_SIGNATURE names none of them"
        )),
    }
}

pub fn spawned(cmdline: &[u8], argv: &[&OsStr]) -> bool {
    let args: Vec<&[u8]> = cmdline
        .strip_suffix(b"\0")
        .unwrap_or(cmdline)
        .split(|&byte: &u8| byte == 0)
        .collect();
    args == argv
        .iter()
        .map(|arg: &&OsStr| arg.as_bytes())
        .collect::<Vec<&[u8]>>()
}

pub fn executable(cargo_messages: &str, target: &str) -> Option<PathBuf> {
    cargo_messages
        .lines()
        .filter_map(|line: &str| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message: &serde_json::Value| {
            message["reason"] == "compiler-artifact" && message["target"]["name"] == target
        })
        .filter_map(|message: serde_json::Value| message["executable"].as_str().map(PathBuf::from))
        .next_back()
}

#[cfg(test)]
mod tests;
