use std::collections::BTreeMap;
use std::io;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::Context;
use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, InputEvent, KeyCode, KeyEvent, RelativeAxisCode, RelativeAxisEvent};
use phonepad_core::input::{Button, InputCommand};
use phonepad_core::keys::{Key, Stroke, Table, typing_plan};
use tokio::sync::{mpsc, watch};

use crate::hypr;
use crate::keymap::{KeyboardLayout, Keymaps};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhoneId(pub u64);

#[derive(Clone, Debug, Default)]
pub struct PhoneIds(Arc<AtomicU64>);

impl PhoneIds {
    pub fn next(&self) -> PhoneId {
        PhoneId(self.0.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Injection {
    Command(InputCommand),
    Move {
        phone: PhoneId,
        seq: u32,
        dx: i32,
        dy: i32,
    },
    Leave(PhoneId),
}

pub type Applied = BTreeMap<PhoneId, u32>;

pub fn applied_seq(applied: &Applied, phone: PhoneId) -> u32 {
    applied.get(&phone).copied().unwrap_or(0)
}

fn mark(applied: &mut Applied, injection: &Injection) -> bool {
    match injection {
        Injection::Command(_) => false,
        Injection::Move { phone, seq, .. } => applied.insert(*phone, *seq) != Some(*seq),
        Injection::Leave(phone) => applied.remove(phone).is_some(),
    }
}

pub fn open_device() -> io::Result<VirtualDevice> {
    let axes: AttributeSet<RelativeAxisCode> = AXES.into_iter().collect();
    let keys: AttributeSet<KeyCode> = device_keys().collect();
    VirtualDevice::builder()?
        .name("phonepad")
        .with_relative_axes(&axes)?
        .with_keys(&keys)?
        .build()
}

pub fn device_keys() -> impl Iterator<Item = KeyCode> {
    KEY_CODES
        .map(KeyCode::new)
        .filter(|code: &KeyCode| format!("{code:?}").starts_with("KEY_"))
        .chain(BUTTONS.map(button_code))
}

pub fn spawn(
    mut device: VirtualDevice,
    applied: watch::Sender<Applied>,
) -> mpsc::Sender<Injection> {
    let (sender, mut receiver): (mpsc::Sender<Injection>, mpsc::Receiver<Injection>) =
        mpsc::channel(QUEUE);
    thread::spawn(move || {
        let mut keymaps: Keymaps = Keymaps::default();
        while let Some(injection) = receiver.blocking_recv() {
            if let Err(err) = inject(&mut device, &mut keymaps, &injection) {
                eprintln!("input {injection:?}: {err}");
            }
            applied.send_if_modified(|applied: &mut Applied| mark(applied, &injection));
        }
    });
    sender
}

pub fn release_buttons() -> [Injection; 3] {
    BUTTONS.map(|button: Button| {
        Injection::Command(InputCommand::Button {
            button,
            pressed: false,
        })
    })
}

fn inject(
    device: &mut VirtualDevice,
    keymaps: &mut Keymaps,
    injection: &Injection,
) -> io::Result<()> {
    match injection {
        Injection::Command(command) => execute(device, keymaps, command),
        Injection::Move { dx, dy, .. } => device.emit(&[
            rel(RelativeAxisCode::REL_X, *dx),
            rel(RelativeAxisCode::REL_Y, *dy),
        ]),
        Injection::Leave(_) => Ok(()),
    }
}

fn execute(
    device: &mut VirtualDevice,
    keymaps: &mut Keymaps,
    command: &InputCommand,
) -> io::Result<()> {
    match command {
        InputCommand::Scroll { dx, dy } => device.emit(&[
            rel(RelativeAxisCode::REL_WHEEL_HI_RES, dy.saturating_neg()),
            rel(RelativeAxisCode::REL_HWHEEL_HI_RES, *dx),
        ]),
        InputCommand::Button { button, pressed } => {
            device.emit(&[key(button_code(*button), i32::from(*pressed))])
        }
        InputCommand::Chord(codes) => chord(device, codes),
        InputCommand::Text(text) => type_text(device, keymaps, text),
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

fn type_text(device: &mut VirtualDevice, keymaps: &mut Keymaps, text: &str) -> io::Result<()> {
    let table: &Table = match hypr::phonepad_keyboard().and_then(|layout: KeyboardLayout| {
        let names: String = format!("{layout:?}");
        keymaps.table(layout).context(names)
    }) {
        Ok(table) => table,
        Err(err) => {
            eprintln!(
                "text of {} chars not typed, no keymap: {err:#}",
                text.chars().count()
            );
            return Ok(());
        }
    };
    for stroke in typing_plan(text, table) {
        match stroke {
            Stroke::Key(key) => tap(device, key)?,
            Stroke::Unicode(run) => wtype(&run),
        }
    }
    Ok(())
}

fn tap(device: &mut VirtualDevice, key: Key) -> io::Result<()> {
    let [press, release]: [Vec<InputEvent>; 2] = tap_events(key);
    device.emit(&press)?;
    device.emit(&release)?;
    thread::sleep(KEY_GAP);
    Ok(())
}

fn tap_events(key: Key) -> [Vec<InputEvent>; 2] {
    let codes: Vec<KeyCode> = key
        .shift
        .into_iter()
        .chain(key.level3)
        .chain([key.code])
        .map(KeyCode::new)
        .collect();
    [
        codes.iter().map(|k: &KeyCode| self::key(*k, 1)).collect(),
        codes
            .iter()
            .rev()
            .map(|k: &KeyCode| self::key(*k, 0))
            .collect(),
    ]
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
    use super::*;

    #[test]
    fn plain_key_is_pressed_and_released_alone() {
        assert_eq!(
            tap_events(Key {
                code: 30,
                shift: None,
                level3: None
            }),
            [vec![key(KeyCode::KEY_A, 1)], vec![key(KeyCode::KEY_A, 0)]]
        );
    }

    #[test]
    fn modifiers_are_held_around_the_key_and_released_in_reverse() {
        assert_eq!(
            tap_events(Key {
                code: 31,
                shift: Some(42),
                level3: Some(100)
            }),
            [
                vec![
                    key(KeyCode::KEY_LEFTSHIFT, 1),
                    key(KeyCode::KEY_RIGHTALT, 1),
                    key(KeyCode::KEY_S, 1)
                ],
                vec![
                    key(KeyCode::KEY_S, 0),
                    key(KeyCode::KEY_RIGHTALT, 0),
                    key(KeyCode::KEY_LEFTSHIFT, 0)
                ]
            ]
        );
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
                Injection::Command(InputCommand::Button {
                    button: Button::Left,
                    pressed: false
                }),
                Injection::Command(InputCommand::Button {
                    button: Button::Middle,
                    pressed: false
                }),
                Injection::Command(InputCommand::Button {
                    button: Button::Right,
                    pressed: false
                }),
            ]
        );
    }

    const PHONE: PhoneId = PhoneId(1);
    const OTHER: PhoneId = PhoneId(2);

    fn moved(phone: PhoneId, seq: u32) -> Injection {
        Injection::Move {
            phone,
            seq,
            dx: 1,
            dy: 0,
        }
    }

    fn marked(start: &[(PhoneId, u32)], injection: &Injection) -> (bool, Vec<(PhoneId, u32)>) {
        let mut applied: Applied = start.iter().copied().collect();
        let changed: bool = mark(&mut applied, injection);
        (changed, applied.into_iter().collect())
    }

    #[test]
    fn each_phone_gets_its_own_id() {
        let ids: PhoneIds = PhoneIds::default();
        assert_eq!([ids.next(), ids.next()], [PhoneId(0), PhoneId(1)]);
    }

    #[test]
    fn a_phone_that_never_moved_has_applied_nothing() {
        assert_eq!(applied_seq(&Applied::new(), PHONE), 0);
    }

    #[test]
    fn a_phone_has_applied_its_own_last_move_not_another_phones() {
        let applied: Applied = [(PHONE, 4), (OTHER, 9)].into_iter().collect();
        assert_eq!(applied_seq(&applied, PHONE), 4);
    }

    #[test]
    fn a_first_move_marks_its_phone() {
        assert_eq!(marked(&[], &moved(PHONE, 1)), (true, vec![(PHONE, 1)]));
    }

    #[test]
    fn a_later_move_replaces_its_phones_mark_and_keeps_the_others() {
        assert_eq!(
            marked(&[(PHONE, 1), (OTHER, 5)], &moved(PHONE, 2)),
            (true, vec![(PHONE, 2), (OTHER, 5)])
        );
    }

    #[test]
    fn the_same_move_again_changes_nothing() {
        assert_eq!(
            marked(&[(PHONE, 2)], &moved(PHONE, 2)),
            (false, vec![(PHONE, 2)])
        );
    }

    #[test]
    fn a_command_marks_nothing() {
        assert_eq!(
            marked(&[(PHONE, 2)], &release_buttons()[0]),
            (false, vec![(PHONE, 2)])
        );
    }

    #[test]
    fn a_leaving_phone_loses_its_mark() {
        assert_eq!(
            marked(&[(PHONE, 2), (OTHER, 5)], &Injection::Leave(PHONE)),
            (true, vec![(OTHER, 5)])
        );
    }

    #[test]
    fn a_phone_that_never_moved_leaves_without_a_change() {
        assert_eq!(
            marked(&[(OTHER, 5)], &Injection::Leave(PHONE)),
            (false, vec![(OTHER, 5)])
        );
    }
}
