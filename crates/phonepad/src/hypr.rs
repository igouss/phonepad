use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use phonepad_core::geometry::{Monitor, Point};
use serde::Deserialize;
use serde::de::DeserializeOwned;

const IPC_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Deserialize)]
struct CursorPos {
    x: i32,
    y: i32,
}

#[derive(Deserialize)]
struct HyprMonitor {
    name: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f64,
    transform: u8,
}

pub fn cursor() -> anyhow::Result<Point> {
    let pos: CursorPos = query("cursorpos")?;
    Ok(Point { x: pos.x, y: pos.y })
}

pub fn monitors() -> anyhow::Result<Vec<Monitor>> {
    let found: Vec<HyprMonitor> = query("monitors")?;
    Ok(found
        .into_iter()
        .filter_map(|m: HyprMonitor| {
            Monitor::from_mode(
                m.name,
                Point { x: m.x, y: m.y },
                m.width,
                m.height,
                m.scale,
                m.transform,
            )
        })
        .collect())
}

fn query<T: DeserializeOwned>(command: &str) -> anyhow::Result<T> {
    let mut stream: UnixStream =
        UnixStream::connect(socket_path()?).context("connect to the Hyprland socket")?;
    stream.set_read_timeout(Some(IPC_TIMEOUT))?;
    stream.set_write_timeout(Some(IPC_TIMEOUT))?;
    stream.write_all(format!("j/{command}").as_bytes())?;
    let mut reply: Vec<u8> = Vec::new();
    stream.read_to_end(&mut reply)?;
    serde_json::from_slice(&reply).with_context(|| format!("hyprctl j/{command} reply"))
}

fn socket_path() -> anyhow::Result<PathBuf> {
    let runtime: String = env::var("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR")?;
    let instance: String =
        env::var("HYPRLAND_INSTANCE_SIGNATURE").context("HYPRLAND_INSTANCE_SIGNATURE")?;
    Ok(PathBuf::from(runtime)
        .join("hypr")
        .join(instance)
        .join(".socket.sock"))
}
