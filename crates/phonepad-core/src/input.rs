#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputCommand {
    Scroll { dx: i32, dy: i32 },
    Button { button: Button, pressed: bool },
    Chord(Vec<u16>),
    Text(String),
}
