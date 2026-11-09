use std::path::PathBuf;

use phonepad_core::decision::Decision;
use phonepad_core::wallpaper::{self, Wallpaper, WallpaperCommand, WallpaperEvent, WallpaperInput};
use serde::Deserialize;
use tokio::process::Command;

const MARKER_DIR: &str = "phonepad";
const MARKER_FILE: &str = "wallpaper-paused";

#[derive(Deserialize)]
struct OweStatus {
    manual_pause: bool,
}

pub struct WallpaperShell {
    state: Wallpaper,
    marker: Option<PathBuf>,
    unavailable_logged: bool,
}

impl WallpaperShell {
    pub fn from_env() -> WallpaperShell {
        WallpaperShell {
            state: Wallpaper::Untouched,
            marker: std::env::var_os("XDG_RUNTIME_DIR")
                .map(|dir| PathBuf::from(dir).join(MARKER_DIR).join(MARKER_FILE)),
            unavailable_logged: false,
        }
    }

    pub async fn startup(&mut self) {
        let Some(marker) = self.marker.as_ref() else {
            self.unavailable("XDG_RUNTIME_DIR is not set");
            return;
        };
        let marker_present: bool = marker.try_exists().unwrap_or(false);
        self.step(WallpaperInput::Startup { marker_present }).await;
    }

    pub async fn capture_started(&mut self) {
        if self.marker.is_none() {
            self.unavailable("XDG_RUNTIME_DIR is not set");
            return;
        }
        match owe(&["status"])
            .await
            .and_then(|out: Vec<u8>| manual_pause(&out))
        {
            Ok(manual_pause) => {
                self.step(WallpaperInput::CaptureStarted { manual_pause })
                    .await
            }
            Err(err) => self.unavailable(&err),
        }
    }

    pub async fn capture_stopped(&mut self) {
        self.step(WallpaperInput::CaptureStopped).await;
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
            WallpaperCommand::Pause => owe(&["pause"]).await.map(|_| ()),
            WallpaperCommand::Resume => owe(&["resume"]).await.map(|_| ()),
            WallpaperCommand::WriteMarker => self.write_marker(),
            WallpaperCommand::RemoveMarker => self.remove_marker(),
        };
        if let Err(err) = done {
            eprintln!("wallpaper {command:?}: {err}");
        }
    }

    fn write_marker(&self) -> Result<(), String> {
        let marker: &PathBuf = self.marker.as_ref().ok_or("no marker path")?;
        if let Some(dir) = marker.parent() {
            std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
        }
        std::fs::write(marker, b"").map_err(|err| err.to_string())
    }

    fn remove_marker(&self) -> Result<(), String> {
        let marker: &PathBuf = self.marker.as_ref().ok_or("no marker path")?;
        std::fs::remove_file(marker).map_err(|err| err.to_string())
    }

    fn unavailable(&mut self, reason: &str) {
        if !self.unavailable_logged {
            eprintln!("wallpaper control unavailable: {reason}");
            self.unavailable_logged = true;
        }
    }
}

async fn owe(args: &[&str]) -> Result<Vec<u8>, String> {
    let output: std::process::Output = Command::new("owe")
        .args(args)
        .output()
        .await
        .map_err(|err| format!("owe {args:?}: {err}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(format!(
            "owe {args:?}: {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn manual_pause(status: &[u8]) -> Result<bool, String> {
    serde_json::from_slice::<OweStatus>(status)
        .map(|status: OweStatus| status.manual_pause)
        .map_err(|err| format!("owe status: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_pause_off() {
        assert_eq!(
            manual_pause(br#"{"status":"ok","paused":false,"manual_pause":false}"#),
            Ok(false)
        );
    }

    #[test]
    fn manual_pause_on() {
        assert_eq!(
            manual_pause(br#"{"status":"ok","paused":true,"manual_pause":true}"#),
            Ok(true)
        );
    }

    #[test]
    fn missing_manual_pause_is_an_error() {
        assert!(manual_pause(br#"{"status":"ok","paused":true}"#).is_err());
    }

    #[test]
    fn empty_status_is_an_error() {
        assert!(manual_pause(b"").is_err());
    }
}
