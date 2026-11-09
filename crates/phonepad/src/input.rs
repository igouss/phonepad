use std::io;
use std::process::Command;
use std::thread;
use std::time::Duration;

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, InputEvent, KeyCode, KeyEvent, RelativeAxisCode, RelativeAxisEvent};
use phonepad_core::input::{Button, InputCommand};
use phonepad_core::keys::{Stroke, typing_plan};
use tokio::sync::mpsc;

const KEY_GAP: Duration = Duration::from_millis(4);
const QUEUE: usize = 256;
const KEY_CODES: std::ops::Range<u16> = 1..0x2FF;
const AXES: [RelativeAxisCode; 6] = [
    RelativeAxisCode::REL_X,
    RelativeAxisCode::REL_Y,
    RelativeAxisCode::REL_WHEEL,
    RelativeAxisCode::REL_HWHEEL,
    RelativeAxisCode::REL_WHEEL_HI_RES,
    RelativeAxisCode::REL_HWHEEL_HI_RES,
];
const BUTTONS: [Button; 3] = [Button::Left, Button::Middle, Button::Right];

pub fn open_device() -> io::Result<VirtualDevice> {
    let axes: AttributeSet<RelativeAxisCode> = AXES.into_iter().collect();
    let keys: AttributeSet<KeyCode> = device_keys().collect();
    VirtualDevice::builder()?
        .name("phonepad")
        .with_relative_axes(&axes)?
        .with_keys(&keys)?
        .build()
}

fn device_keys() -> impl Iterator<Item = KeyCode> {
    KEY_CODES
        .map(KeyCode::new)
        .filter(|code: &KeyCode| format!("{code:?}").starts_with("KEY_"))
        .chain(BUTTONS.map(button_code))
}

pub fn spawn(mut device: VirtualDevice) -> mpsc::Sender<InputCommand> {
    let (sender, mut receiver): (mpsc::Sender<InputCommand>, mpsc::Receiver<InputCommand>) =
        mpsc::channel(QUEUE);
    thread::spawn(move || {
        while let Some(command) = receiver.blocking_recv() {
            if let Err(err) = execute(&mut device, &command) {
                eprintln!("input {command:?}: {err}");
            }
        }
    });
    sender
}

pub fn release_buttons() -> [InputCommand; 3] {
    BUTTONS.map(|button: Button| InputCommand::Button {
        button,
        pressed: false,
    })
}

fn execute(device: &mut VirtualDevice, command: &InputCommand) -> io::Result<()> {
    match command {
        InputCommand::Move { dx, dy } => device.emit(&[
            rel(RelativeAxisCode::REL_X, *dx),
            rel(RelativeAxisCode::REL_Y, *dy),
        ]),
        InputCommand::Scroll { dx, dy } => device.emit(&[
            rel(RelativeAxisCode::REL_WHEEL_HI_RES, dy.saturating_neg()),
            rel(RelativeAxisCode::REL_HWHEEL_HI_RES, *dx),
        ]),
        InputCommand::Button { button, pressed } => {
            device.emit(&[key(button_code(*button), i32::from(*pressed))])
        }
        InputCommand::Chord(codes) => chord(device, codes),
        InputCommand::Text(text) => type_text(device, text),
    }
}

fn chord(device: &mut VirtualDevice, codes: &[u16]) -> io::Result<()> {
    for code in codes {
        device.emit(&[key(KeyCode::new(*code), 1)])?;
    }
    for code in codes.iter().rev() {
        device.emit(&[key(KeyCode::new(*code), 0)])?;
    }
    Ok(())
}

fn type_text(device: &mut VirtualDevice, text: &str) -> io::Result<()> {
    for stroke in typing_plan(text) {
        match stroke {
            Stroke::Key { code, shift } => tap(device, KeyCode::new(code), shift)?,
            Stroke::Unicode(run) => wtype(&run),
        }
    }
    Ok(())
}

fn tap(device: &mut VirtualDevice, code: KeyCode, shift: bool) -> io::Result<()> {
    let shift_key: Option<KeyCode> = shift.then_some(KeyCode::KEY_LEFTSHIFT);
    let press: Vec<InputEvent> = shift_key
        .map(|k: KeyCode| key(k, 1))
        .into_iter()
        .chain([key(code, 1)])
        .collect();
    let release: Vec<InputEvent> = [key(code, 0)]
        .into_iter()
        .chain(shift_key.map(|k: KeyCode| key(k, 0)))
        .collect();
    device.emit(&press)?;
    device.emit(&release)?;
    thread::sleep(KEY_GAP);
    Ok(())
}

fn wtype(run: &str) {
    match Command::new("wtype").arg("--").arg(run).status() {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("wtype {run:?}: {status}"),
        Err(err) => eprintln!("wtype {run:?}: {err}"),
    }
}

fn button_code(button: Button) -> KeyCode {
    match button {
        Button::Left => KeyCode::BTN_LEFT,
        Button::Middle => KeyCode::BTN_MIDDLE,
        Button::Right => KeyCode::BTN_RIGHT,
    }
}

fn key(code: KeyCode, value: i32) -> InputEvent {
    *KeyEvent::new(code, value)
}

fn rel(axis: RelativeAxisCode, value: i32) -> InputEvent {
    *RelativeAxisEvent::new(axis, value)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    const PRINTABLE_ASCII: &str = " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~";

    const US_LAYOUT: [(&str, bool); 95] = [
        ("SPACE", false),
        ("1", true),
        ("APOSTROPHE", true),
        ("3", true),
        ("4", true),
        ("5", true),
        ("7", true),
        ("APOSTROPHE", false),
        ("9", true),
        ("0", true),
        ("8", true),
        ("EQUAL", true),
        ("COMMA", false),
        ("MINUS", false),
        ("DOT", false),
        ("SLASH", false),
        ("0", false),
        ("1", false),
        ("2", false),
        ("3", false),
        ("4", false),
        ("5", false),
        ("6", false),
        ("7", false),
        ("8", false),
        ("9", false),
        ("SEMICOLON", true),
        ("SEMICOLON", false),
        ("COMMA", true),
        ("EQUAL", false),
        ("DOT", true),
        ("SLASH", true),
        ("2", true),
        ("A", true),
        ("B", true),
        ("C", true),
        ("D", true),
        ("E", true),
        ("F", true),
        ("G", true),
        ("H", true),
        ("I", true),
        ("J", true),
        ("K", true),
        ("L", true),
        ("M", true),
        ("N", true),
        ("O", true),
        ("P", true),
        ("Q", true),
        ("R", true),
        ("S", true),
        ("T", true),
        ("U", true),
        ("V", true),
        ("W", true),
        ("X", true),
        ("Y", true),
        ("Z", true),
        ("LEFTBRACE", false),
        ("BACKSLASH", false),
        ("RIGHTBRACE", false),
        ("6", true),
        ("MINUS", true),
        ("GRAVE", false),
        ("A", false),
        ("B", false),
        ("C", false),
        ("D", false),
        ("E", false),
        ("F", false),
        ("G", false),
        ("H", false),
        ("I", false),
        ("J", false),
        ("K", false),
        ("L", false),
        ("M", false),
        ("N", false),
        ("O", false),
        ("P", false),
        ("Q", false),
        ("R", false),
        ("S", false),
        ("T", false),
        ("U", false),
        ("V", false),
        ("W", false),
        ("X", false),
        ("Y", false),
        ("Z", false),
        ("LEFTBRACE", true),
        ("BACKSLASH", true),
        ("RIGHTBRACE", true),
        ("GRAVE", true),
    ];

    fn us_stroke(entry: &(&str, bool)) -> Stroke {
        Stroke::Key {
            code: KeyCode::from_str(&format!("KEY_{}", entry.0))
                .unwrap()
                .code(),
            shift: entry.1,
        }
    }

    #[test]
    fn printable_ascii_types_as_the_us_layout_keys_named_by_evdev() {
        let expected: Vec<(char, Stroke)> = PRINTABLE_ASCII
            .chars()
            .zip(US_LAYOUT.iter().map(us_stroke))
            .collect();
        let actual: Vec<(char, Stroke)> = PRINTABLE_ASCII
            .chars()
            .map(|ch: char| (ch, typing_plan(&ch.to_string())))
            .map(|(ch, plan): (char, Vec<Stroke>)| (ch, plan.into_iter().next().unwrap()))
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn printable_ascii_is_all_95_characters_in_order() {
        assert_eq!(
            PRINTABLE_ASCII,
            (32u8..=126).map(char::from).collect::<String>()
        );
    }

    #[test]
    fn whole_ascii_string_plans_one_stroke_per_character() {
        assert_eq!(typing_plan(PRINTABLE_ASCII).len(), 95);
    }

    #[test]
    fn device_key_set_excludes_button_codes_that_would_reclassify_it() {
        let keys: Vec<KeyCode> = device_keys().collect();
        assert_eq!(
            (
                keys.contains(&KeyCode::KEY_A),
                keys.contains(&KeyCode::BTN_LEFT),
                keys.contains(&KeyCode::BTN_TOUCH)
            ),
            (true, true, false)
        );
    }

    #[test]
    fn release_buttons_releases_all_three() {
        assert_eq!(
            release_buttons(),
            [
                InputCommand::Button {
                    button: Button::Left,
                    pressed: false
                },
                InputCommand::Button {
                    button: Button::Middle,
                    pressed: false
                },
                InputCommand::Button {
                    button: Button::Right,
                    pressed: false
                },
            ]
        );
    }
}
