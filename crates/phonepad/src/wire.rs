use std::num::NonZeroU32;
use std::str::FromStr;

use evdev::KeyCode;
use phonepad_core::geometry::{Point, Size};
use phonepad_core::input::{Button, InputCommand};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum WireMessage {
    Move { dx: i32, dy: i32 },
    Scroll { dx: i32, dy: i32 },
    Button { b: WireButton, down: u8 },
    Keys { k: Vec<String> },
    Text { s: String },
    Visible { v: bool },
    View { w: u32, h: u32 },
    Ack,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum WireButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhoneMessage {
    Input(InputCommand),
    Visible(bool),
    View(Option<Size>),
    Ack,
}

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("malformed message: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("unknown key name {0:?}")]
    UnknownKey(String),
    #[error("button state {0} is neither 0 nor 1")]
    ButtonState(u8),
}

pub fn parse(text: &str) -> Result<PhoneMessage, WireError> {
    let message: WireMessage = serde_json::from_str(text)?;
    Ok(match message {
        WireMessage::Move { dx, dy } => PhoneMessage::Input(InputCommand::Move { dx, dy }),
        WireMessage::Scroll { dx, dy } => PhoneMessage::Input(InputCommand::Scroll { dx, dy }),
        WireMessage::Button { b, down } => PhoneMessage::Input(InputCommand::Button {
            button: button(b),
            pressed: pressed(down)?,
        }),
        WireMessage::Keys { k } => PhoneMessage::Input(InputCommand::Chord(
            k.iter()
                .map(|name: &String| key_code(name))
                .collect::<Result<Vec<u16>, WireError>>()?,
        )),
        WireMessage::Text { s } => PhoneMessage::Input(InputCommand::Text(s)),
        WireMessage::Visible { v } => PhoneMessage::Visible(v),
        WireMessage::View { w, h } => PhoneMessage::View(size(w, h)),
        WireMessage::Ack => PhoneMessage::Ack,
    })
}

pub fn key_code(name: &str) -> Result<u16, WireError> {
    name.strip_prefix("KEY_")
        .and_then(|_| KeyCode::from_str(name).ok())
        .map(|code: KeyCode| code.code())
        .ok_or_else(|| WireError::UnknownKey(name.to_owned()))
}

pub fn frame_message(region: Point, cursor: Point, jpeg: &[u8]) -> Vec<u8> {
    [region.x, region.y, cursor.x, cursor.y]
        .iter()
        .flat_map(|value: &i32| value.to_be_bytes())
        .chain(jpeg.iter().copied())
        .collect()
}

fn button(wire: WireButton) -> Button {
    match wire {
        WireButton::Left => Button::Left,
        WireButton::Middle => Button::Middle,
        WireButton::Right => Button::Right,
    }
}

fn pressed(down: u8) -> Result<bool, WireError> {
    match down {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(WireError::ButtonState(other)),
    }
}

fn size(w: u32, h: u32) -> Option<Size> {
    Some(Size {
        w: NonZeroU32::new(w)?,
        h: NonZeroU32::new(h)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(text: &str) -> Result<PhoneMessage, String> {
        parse(text).map_err(|err: WireError| err.to_string())
    }

    fn command(command: InputCommand) -> Result<PhoneMessage, String> {
        Ok(PhoneMessage::Input(command))
    }

    #[test]
    fn move_carries_signed_deltas() {
        assert_eq!(
            input(r#"{"t":"move","dx":-3,"dy":7}"#),
            command(InputCommand::Move { dx: -3, dy: 7 })
        );
    }

    #[test]
    fn scroll_carries_signed_deltas() {
        assert_eq!(
            input(r#"{"t":"scroll","dx":2,"dy":-5}"#),
            command(InputCommand::Scroll { dx: 2, dy: -5 })
        );
    }

    #[test]
    fn button_down_one_presses() {
        assert_eq!(
            input(r#"{"t":"button","b":"middle","down":1}"#),
            command(InputCommand::Button {
                button: Button::Middle,
                pressed: true
            })
        );
    }

    #[test]
    fn button_down_zero_releases() {
        assert_eq!(
            input(r#"{"t":"button","b":"right","down":0}"#),
            command(InputCommand::Button {
                button: Button::Right,
                pressed: false
            })
        );
    }

    #[test]
    fn button_down_two_is_rejected() {
        assert!(matches!(
            parse(r#"{"t":"button","b":"left","down":2}"#),
            Err(WireError::ButtonState(2))
        ));
    }

    #[test]
    fn unknown_button_is_malformed() {
        assert!(matches!(
            parse(r#"{"t":"button","b":"side","down":1}"#),
            Err(WireError::Malformed(_))
        ));
    }

    #[test]
    fn keys_zero_is_an_empty_chord() {
        assert_eq!(
            input(r#"{"t":"keys","k":[]}"#),
            command(InputCommand::Chord(vec![]))
        );
    }

    #[test]
    fn keys_one_maps_name_to_code() {
        assert_eq!(
            input(r#"{"t":"keys","k":["KEY_ENTER"]}"#),
            command(InputCommand::Chord(vec![28]))
        );
    }

    #[test]
    fn keys_many_keep_order() {
        assert_eq!(
            input(r#"{"t":"keys","k":["KEY_LEFTCTRL","KEY_LEFTSHIFT","KEY_T"]}"#),
            command(InputCommand::Chord(vec![29, 42, 20]))
        );
    }

    #[test]
    fn unknown_key_name_is_rejected() {
        assert!(matches!(
            parse(r#"{"t":"keys","k":["KEY_A","KEY_FOOBAR"]}"#),
            Err(WireError::UnknownKey(name)) if name == "KEY_FOOBAR"
        ));
    }

    #[test]
    fn button_names_are_not_keys() {
        assert!(matches!(
            parse(r#"{"t":"keys","k":["BTN_LEFT"]}"#),
            Err(WireError::UnknownKey(_))
        ));
    }

    #[test]
    fn text_keeps_the_string() {
        assert_eq!(
            input(r#"{"t":"text","s":"aé€"}"#),
            command(InputCommand::Text("aé€".to_owned()))
        );
    }

    #[test]
    fn visible_true() {
        assert_eq!(
            parse(r#"{"t":"visible","v":true}"#).unwrap(),
            PhoneMessage::Visible(true)
        );
    }

    #[test]
    fn visible_false() {
        assert_eq!(
            parse(r#"{"t":"visible","v":false}"#).unwrap(),
            PhoneMessage::Visible(false)
        );
    }

    #[test]
    fn view_with_area_is_a_size() {
        assert_eq!(
            parse(r#"{"t":"view","w":647,"h":1071}"#).unwrap(),
            PhoneMessage::View(Some(Size {
                w: NonZeroU32::new(647).unwrap(),
                h: NonZeroU32::new(1071).unwrap()
            }))
        );
    }

    #[test]
    fn view_with_zero_width_is_no_size() {
        assert_eq!(
            parse(r#"{"t":"view","w":0,"h":1071}"#).unwrap(),
            PhoneMessage::View(None)
        );
    }

    #[test]
    fn ack() {
        assert_eq!(parse(r#"{"t":"ack"}"#).unwrap(), PhoneMessage::Ack);
    }

    #[test]
    fn malformed_json_is_an_error() {
        assert!(matches!(
            parse(r#"{"t":"move","dx":"#),
            Err(WireError::Malformed(_))
        ));
    }

    #[test]
    fn unknown_type_is_an_error() {
        assert!(matches!(
            parse(r#"{"t":"teleport"}"#),
            Err(WireError::Malformed(_))
        ));
    }

    #[test]
    fn missing_type_is_an_error() {
        assert!(matches!(parse(r#"{"dx":1}"#), Err(WireError::Malformed(_))));
    }

    #[test]
    fn missing_field_is_an_error() {
        assert!(matches!(
            parse(r#"{"t":"move","dx":1}"#),
            Err(WireError::Malformed(_))
        ));
    }

    #[test]
    fn fractional_delta_is_an_error() {
        assert!(matches!(
            parse(r#"{"t":"move","dx":1.5,"dy":0}"#),
            Err(WireError::Malformed(_))
        ));
    }

    #[test]
    fn frame_header_is_four_big_endian_words_before_the_jpeg() {
        assert_eq!(
            frame_message(Point { x: 1, y: -2 }, Point { x: 258, y: 3 }, &[0xFF, 0xD8]),
            vec![
                0, 0, 0, 1, 0xFF, 0xFF, 0xFF, 0xFE, 0, 0, 1, 2, 0, 0, 0, 3, 0xFF, 0xD8
            ]
        );
    }
}
