use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Context;
use phonepad_core::geometry::{Monitor, Point};

use crate::keymap::KeyboardLayout;
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

const KEYBOARD: &str = "phonepad";

#[derive(Deserialize)]
struct Devices {
    keyboards: Vec<HyprKeyboard>,
}

#[derive(Deserialize)]
struct HyprKeyboard {
    name: String,
    #[serde(flatten)]
    layout: KeyboardLayout,
}

pub fn phonepad_keyboard() -> anyhow::Result<KeyboardLayout> {
    keyboard(query("devices")?)
}

fn keyboard(devices: Devices) -> anyhow::Result<KeyboardLayout> {
    devices
        .keyboards
        .into_iter()
        .find(|keyboard: &HyprKeyboard| keyboard.name == KEYBOARD)
        .map(|keyboard: HyprKeyboard| keyboard.layout)
        .with_context(|| format!("Hyprland has no keyboard named {KEYBOARD}"))
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

static SOCKET: OnceLock<PathBuf> = OnceLock::new();

fn socket_path() -> anyhow::Result<PathBuf> {
    if let Some(path) = SOCKET.get() {
        return Ok(path.clone());
    }
    let path: PathBuf = resolve_socket_path()?;
    Ok(SOCKET.get_or_init(|| path).clone())
}

fn resolve_socket_path() -> anyhow::Result<PathBuf> {
    let runtime: String = env::var("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR")?;
    let instance: String =
        env::var("HYPRLAND_INSTANCE_SIGNATURE").context("HYPRLAND_INSTANCE_SIGNATURE")?;
    Ok(PathBuf::from(runtime)
        .join("hypr")
        .join(instance)
        .join(".socket.sock"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FCITX: &str = r#"{"address": "0x1", "name": "hl-virtual-keyboard-fcitx5", "rules": "", "model": "", "layout": "us", "variant": "", "options": "", "active_layout_index": 0, "active_keymap": "English (US)", "capsLock": false, "numLock": true, "main": true}"#;
    const PHONEPAD: &str = r#"{"address": "0x2", "name": "phonepad", "rules": "evdev", "model": "pc105", "layout": "us,de", "variant": ",nodeadkeys", "options": "omarchy:shift_caps_compose", "active_layout_index": 1, "active_keymap": "German", "capsLock": false, "numLock": true, "main": false}"#;

    fn devices(keyboards: &[&str]) -> anyhow::Result<Devices> {
        Ok(serde_json::from_str(&format!(
            r#"{{"mice": [], "tablets": [], "touch": [], "switches": [], "keyboards": [{}]}}"#,
            keyboards.join(",")
        ))?)
    }

    #[test]
    fn phonepad_entry_supplies_the_layout_not_the_main_keyboard() -> anyhow::Result<()> {
        assert_eq!(
            keyboard(devices(&[FCITX, PHONEPAD])?)?,
            KeyboardLayout {
                rules: String::from("evdev"),
                model: String::from("pc105"),
                layout: String::from("us,de"),
                variant: String::from(",nodeadkeys"),
                options: String::from("omarchy:shift_caps_compose"),
                active_layout_index: 1,
            }
        );
        Ok(())
    }

    #[test]
    fn no_phonepad_entry_is_an_error() -> anyhow::Result<()> {
        assert!(keyboard(devices(&[FCITX])?).is_err());
        Ok(())
    }

    #[test]
    fn no_keyboards_is_an_error() -> anyhow::Result<()> {
        assert!(keyboard(devices(&[])?).is_err());
        Ok(())
    }
}
