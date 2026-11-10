use crate::testbed::{Compositor, spawned};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const STOCK: &str = "/usr/bin/Hyprland";
const START_TIMEOUT: Duration = Duration::from_secs(20);
const STOP_PATIENCE: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(100);
const LOG_TAIL: usize = 10;
const CONFIG: &str = "monitor = , preferred, auto, 1
misc {
    disable_hyprland_logo = true
    disable_splash_rendering = true
    force_default_wallpaper = 0
    disable_autoreload = true
    background_color = rgb(000000)
}
animations {
    enabled = false
}
decoration {
    rounding = 0
    blur {
        enabled = false
    }
    shadow {
        enabled = false
    }
}
general {
    gaps_in = 0
    gaps_out = 0
    border_size = 0
}
ecosystem {
    no_update_news = true
    no_donation_nag = true
}
";
const HOST_POINTER_OFF: &str = "keyword device[wl_pointer]:enabled false";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Output {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub at: (i64, i64),
    pub size: (i64, i64),
}

pub struct Nested {
    child: Child,
    argv: [OsString; 3],
    log: PathBuf,
    pub compositor: Compositor,
    socket: PathBuf,
}

pub fn lock(text: &str) -> Option<(u32, String)> {
    let mut lines: std::str::Lines = text.lines();
    let pid: u32 = lines.next()?.trim().parse().ok()?;
    let display: &str = lines.next()?.trim();
    (!display.is_empty()).then(|| (pid, display.to_owned()))
}

pub fn output(monitors: &str) -> Result<Output, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(monitors).map_err(|e| format!("monitors: {e}"))?;
    match parsed.as_array().map(Vec::as_slice) {
        Some([monitor]) => {
            let dimension = |key: &str| -> Result<u32, String> {
                monitor[key]
                    .as_u64()
                    .and_then(|value: u64| u32::try_from(value).ok())
                    .ok_or_else(|| format!("monitor has no {key}: {monitor}"))
            };
            Ok(Output {
                width: dimension("width")?,
                height: dimension("height")?,
                refresh_hz: monitor["refreshRate"]
                    .as_f64()
                    .ok_or_else(|| format!("monitor has no refreshRate: {monitor}"))?,
            })
        }
        Some(others) => Err(format!("expected one monitor, found {}", others.len())),
        None => Err(format!("monitors is not a list: {monitors}")),
    }
}

pub fn placed(clients: &str) -> Result<Vec<Placed>, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(clients).map_err(|e| format!("clients: {e}"))?;
    let pair = |value: &serde_json::Value| -> Option<(i64, i64)> {
        Some((value.get(0)?.as_i64()?, value.get(1)?.as_i64()?))
    };
    parsed
        .as_array()
        .ok_or_else(|| format!("clients is not a list: {clients}"))?
        .iter()
        .map(|client: &serde_json::Value| {
            pair(&client["at"])
                .zip(pair(&client["size"]))
                .map(|(at, size): ((i64, i64), (i64, i64))| Placed { at, size })
                .ok_or_else(|| format!("client without at and size: {client}"))
        })
        .collect()
}

fn found(hypr: &Path, pid: u32) -> Option<(String, String)> {
    std::fs::read_dir(hypr)
        .ok()?
        .filter_map(Result::ok)
        .find_map(|entry: std::fs::DirEntry| {
            let text: String = std::fs::read_to_string(entry.path().join("hyprland.lock")).ok()?;
            let (owner, display): (u32, String) = lock(&text)?;
            (owner == pid).then(|| (entry.file_name().to_string_lossy().into_owned(), display))
        })
}

fn tail(log: &Path) -> String {
    let text: String = std::fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(LOG_TAIL)..].join("\n")
}

impl Nested {
    pub fn start(binary: &Path, scratch: &Path) -> Result<Nested, String> {
        let parent: PathBuf = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .ok_or("XDG_RUNTIME_DIR is not set")?;
        let host: String =
            std::env::var("WAYLAND_DISPLAY").map_err(|e| format!("WAYLAND_DISPLAY: {e}"))?;
        let config: PathBuf = scratch.join("hyprland.conf");
        std::fs::write(&config, CONFIG).map_err(|e| format!("{}: {e}", config.display()))?;
        let log: PathBuf = scratch.join("hyprland.log");
        let out: File = File::create(&log).map_err(|e| format!("{}: {e}", log.display()))?;
        let err: File = out.try_clone().map_err(|e| e.to_string())?;
        let argv: [OsString; 3] = [
            binary.as_os_str().to_owned(),
            OsString::from("-c"),
            config.into_os_string(),
        ];
        let child: Child = Command::new(&argv[0])
            .args(&argv[1..])
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env("WAYLAND_DISPLAY", &host)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn()
            .map_err(|e| format!("spawn {}: {e}", binary.display()))?;
        let mut nested: Nested = Nested {
            child,
            argv,
            log,
            compositor: Compositor {
                display: String::new(),
                signature: String::new(),
            },
            socket: PathBuf::new(),
        };
        let start: Instant = Instant::now();
        let pid: u32 = nested.child.id();
        loop {
            if let Some(status) = nested.child.try_wait().map_err(|e| e.to_string())? {
                return Err(format!(
                    "nested Hyprland exited {status} before its socket listened:\n{}",
                    tail(&nested.log)
                ));
            }
            if let Some((signature, display)) = found(&parent.join("hypr"), pid) {
                let socket: PathBuf = parent.join("hypr").join(&signature).join(".socket.sock");
                if UnixStream::connect(&socket).is_ok() && parent.join(&display).exists() {
                    nested.compositor = Compositor { display, signature };
                    nested.socket = socket;
                    nested.request(HOST_POINTER_OFF)?;
                    return Ok(nested);
                }
            }
            if start.elapsed() > START_TIMEOUT {
                return Err(format!(
                    "nested Hyprland (PID {pid}) did not listen within {START_TIMEOUT:?}:\n{}",
                    tail(&nested.log)
                ));
            }
            thread::sleep(POLL);
        }
    }

    pub fn request(&self, command: &str) -> Result<String, String> {
        let mut socket: UnixStream = UnixStream::connect(&self.socket)
            .map_err(|e| format!("{}: {e}", self.socket.display()))?;
        socket
            .write_all(command.as_bytes())
            .map_err(|e| format!("{command}: {e}"))?;
        let mut reply: String = String::new();
        socket
            .read_to_string(&mut reply)
            .map_err(|e| format!("{command}: {e}"))?;
        Ok(reply)
    }

    pub fn output(&self) -> Result<Output, String> {
        output(&self.request("j/monitors")?)
    }

    pub fn clients(&self) -> Result<Vec<Placed>, String> {
        placed(&self.request("j/clients")?)
    }

    pub fn park(&self, (x, y): (u32, u32)) -> Result<(), String> {
        match self
            .request(&format!("dispatch movecursor {x} {y}"))?
            .as_str()
        {
            "ok" => Ok(()),
            other => Err(format!("movecursor {x} {y}: {other}")),
        }
    }

    pub fn wayland_socket(&self) -> Result<PathBuf, String> {
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(|parent: OsString| PathBuf::from(parent).join(&self.compositor.display))
            .ok_or_else(|| String::from("XDG_RUNTIME_DIR is not set"))
    }

    fn owned(&self) -> Result<u32, String> {
        let pid: u32 = self.child.id();
        let cmdline: Vec<u8> = std::fs::read(format!("/proc/{pid}/cmdline"))
            .map_err(|e| format!("/proc/{pid}/cmdline: {e}"))?;
        let argv: Vec<&OsStr> = self.argv.iter().map(OsString::as_os_str).collect();
        if spawned(&cmdline, &argv) {
            Ok(pid)
        } else {
            Err(format!(
                "PID {pid} is not the nested Hyprland this run spawned"
            ))
        }
    }

    pub fn stop(mut self) -> Result<Option<i32>, String> {
        self.terminate()
    }

    fn terminate(&mut self) -> Result<Option<i32>, String> {
        if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
            return Ok(status.code());
        }
        let pid: u32 = self.owned()?;
        let sent: ExitStatus = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .map_err(|e| format!("kill -TERM {pid}: {e}"))?;
        if !sent.success() {
            return Err(format!("kill -TERM {pid}: {sent}"));
        }
        let start: Instant = Instant::now();
        while start.elapsed() < STOP_PATIENCE {
            if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
                return Ok(status.code());
            }
            thread::sleep(POLL);
        }
        self.child.kill().map_err(|e| format!("kill {pid}: {e}"))?;
        self.child.wait().map_err(|e| format!("wait {pid}: {e}"))?;
        Err(format!(
            "nested Hyprland (PID {pid}) ignored SIGTERM for {STOP_PATIENCE:?}"
        ))
    }
}

impl Drop for Nested {
    fn drop(&mut self) {
        if let Err(error) = self.terminate() {
            eprintln!("{error}");
        }
    }
}

#[cfg(test)]
mod tests;
