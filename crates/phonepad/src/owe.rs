use std::fs::{File, OpenOptions, TryLockError};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use phonepad_core::decision::Decision;
use phonepad_core::wallpaper::{
    self, Marker, Observation, Wallpaper, WallpaperCommand, WallpaperEvent, WallpaperInput,
};
use serde::Deserialize;

use crate::tool;

const MARKER_DIR: &str = "phonepad";
const MARKER_FILE: &str = "wallpaper-paused";

#[derive(Deserialize)]
struct OweStatus {
    manual_pause: bool,
}

pub struct WallpaperShell {
    owe: String,
    state: Wallpaper,
    marker: Option<PathBuf>,
    held: Option<File>,
    unavailable_logged: bool,
}

impl WallpaperShell {
    pub fn from_env() -> WallpaperShell {
        WallpaperShell {
            owe: String::from("owe"),
            state: Wallpaper::Idle,
            marker: std::env::var_os("XDG_RUNTIME_DIR")
                .map(|dir| PathBuf::from(dir).join(MARKER_DIR).join(MARKER_FILE)),
            held: None,
            unavailable_logged: false,
        }
    }

    pub async fn startup(&mut self) {
        match self.marker_path().and_then(|path: &Path| probe(path)) {
            Ok(marker) => self.step(WallpaperInput::Startup { marker }).await,
            Err(err) => self.unavailable(&err),
        }
    }

    pub async fn capture_started(&mut self) {
        match self.observe().await {
            Ok(observation) => self.step(WallpaperInput::CaptureStarted(observation)).await,
            Err(err) => self.unavailable(&err),
        }
    }

    pub async fn tick(&mut self) {
        match (self.observe().await, self.state) {
            (Ok(observation), Wallpaper::Idle) => {
                self.step(WallpaperInput::CaptureStarted(observation)).await
            }
            (Ok(observation), Wallpaper::Yielded | Wallpaper::Held) => {
                self.step(WallpaperInput::Observed(observation)).await
            }
            (Err(err), Wallpaper::Idle | Wallpaper::Yielded | Wallpaper::Held) => {
                self.unavailable(&err)
            }
        }
    }

    pub async fn capture_stopped(&mut self) {
        self.step(WallpaperInput::CaptureStopped).await;
    }

    async fn observe(&self) -> Result<Observation, String> {
        let marker: Marker = probe(self.marker_path()?)?;
        let status: Vec<u8> = tool::run(&self.owe, &["status"]).await?;
        Ok(Observation {
            manual_pause: manual_pause(&status)?,
            marker,
        })
    }

    async fn step(&mut self, input: WallpaperInput) {
        let decision: Decision<WallpaperEvent, WallpaperCommand> =
            wallpaper::decide(&self.state, input);
        self.state = decision.events.iter().fold(self.state, wallpaper::evolve);
        for command in decision.commands {
            self.execute(command).await;
        }
    }

    async fn execute(&mut self, command: WallpaperCommand) {
        let done: Result<(), String> = match command {
            WallpaperCommand::Pause => tool::run(&self.owe, &["pause"]).await.map(|_| ()),
            WallpaperCommand::Resume => tool::run(&self.owe, &["resume"]).await.map(|_| ()),
            WallpaperCommand::TakeMarker => self.take_marker(),
            WallpaperCommand::ClearMarker => self.clear_marker(),
        };
        if let Err(err) = done {
            eprintln!("wallpaper {command:?}: {err}");
        }
    }

    fn marker_path(&self) -> Result<&Path, String> {
        self.marker
            .as_deref()
            .ok_or_else(|| String::from("XDG_RUNTIME_DIR is not set"))
    }

    fn take_marker(&mut self) -> Result<(), String> {
        if self.held.is_none() {
            self.held = Some(take(self.marker_path()?)?);
        }
        Ok(())
    }

    fn clear_marker(&mut self) -> Result<(), String> {
        self.take_marker()?;
        self.held.take().map_or(Ok(()), |file: File| {
            file.set_len(0)
                .and_then(|()| file.unlock())
                .map_err(|err| err.to_string())
        })
    }

    fn unavailable(&mut self, reason: &str) {
        if !self.unavailable_logged {
            eprintln!("wallpaper control unavailable: {reason}");
            self.unavailable_logged = true;
        }
    }
}

fn probe(path: &Path) -> Result<Marker, String> {
    let fail = |err: std::io::Error| format!("{}: {err}", path.display());
    let file: File = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Marker::Absent),
        Err(err) => return Err(fail(err)),
    };
    match file.try_lock() {
        Ok(()) => {
            let empty: bool = file.metadata().map_err(fail)?.len() == 0;
            file.unlock().map_err(fail)?;
            Ok(if empty { Marker::Absent } else { Marker::Stale })
        }
        Err(TryLockError::WouldBlock) => Ok(Marker::Live),
        Err(TryLockError::Error(err)) => Err(fail(err)),
    }
}

fn take(path: &Path) -> Result<File, String> {
    let fail = |err: std::io::Error| format!("{}: {err}", path.display());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    let mut file: File = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(fail)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            return Err(format!("{}: held by another phonepad", path.display()));
        }
        Err(TryLockError::Error(err)) => return Err(fail(err)),
    }
    file.set_len(0).map_err(fail)?;
    writeln!(file, "{}", std::process::id()).map_err(fail)?;
    Ok(file)
}

fn manual_pause(status: &[u8]) -> Result<bool, String> {
    serde_json::from_slice::<OweStatus>(status)
        .map(|status: OweStatus| status.manual_pause)
        .map_err(|err| format!("owe status: {err}"))
}

#[cfg(test)]
mod tests;
