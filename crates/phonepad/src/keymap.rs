use evdev::KeyCode;
use phonepad_core::keys::{Key, Table};
use serde::Deserialize;
use xkbcommon::xkb;

const EVDEV_OFFSET: u32 = 8;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct KeyboardLayout {
    pub rules: String,
    pub model: String,
    pub layout: String,
    pub variant: String,
    pub options: String,
    pub active_layout_index: u32,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeymapError {
    #[error("layout names contain a NUL byte")]
    Nul,
    #[error("libxkbcommon cannot compile the layout")]
    Compile,
    #[error("active layout {active} out of {count}")]
    NoGroup { active: u32, count: u32 },
}

#[derive(Default)]
pub struct Keymaps {
    cached: Option<(KeyboardLayout, Table)>,
}

impl Keymaps {
    pub fn table(&mut self, layout: KeyboardLayout) -> Result<&Table, KeymapError> {
        let table: Table = match self.cached.take() {
            Some((cached, table)) if cached == layout => table,
            Some(_) | None => compile(&layout)?,
        };
        let (_, table): &mut (KeyboardLayout, Table) = self.cached.insert((layout, table));
        Ok(table)
    }
}

pub fn compile(layout: &KeyboardLayout) -> Result<Table, KeymapError> {
    let names: [&str; 5] = [
        &layout.rules,
        &layout.model,
        &layout.layout,
        &layout.variant,
        &layout.options,
    ];
    if names.iter().any(|name: &&str| name.contains('\0')) {
        return Err(KeymapError::Nul);
    }
    let context: xkb::Context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap: xkb::Keymap = xkb::Keymap::new_from_names(
        &context,
        &layout.rules,
        &layout.model,
        &layout.layout,
        &layout.variant,
        Some(layout.options.clone()),
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .ok_or(KeymapError::Compile)?;
    let group: u32 = layout.active_layout_index;
    let count: u32 = keymap.num_layouts();
    if group >= count {
        return Err(KeymapError::NoGroup {
            active: group,
            count,
        });
    }
    Ok(table(&keymap, group))
}

fn table(keymap: &xkb::Keymap, group: u32) -> Table {
    let keys: Vec<u16> = typing_keys();
    let base: xkb::State = pressed(keymap, group, &[]);
    let shift: Option<u16> = key_for(&base, &keys, xkb::keysyms::KEY_Shift_L);
    let level3: Option<u16> = key_for(&base, &keys, xkb::keysyms::KEY_ISO_Level3_Shift);
    let levels: [Option<(Option<u16>, Option<u16>)>; 4] = [
        Some((None, None)),
        shift.map(|s: u16| (Some(s), None)),
        level3.map(|l: u16| (None, Some(l))),
        shift
            .zip(level3)
            .map(|(s, l): (u16, u16)| (Some(s), Some(l))),
    ];
    let mut table: Table = Table::new();
    for (shift, level3) in levels.into_iter().flatten() {
        let held: Vec<u16> = shift.into_iter().chain(level3).collect();
        let state: xkb::State = pressed(keymap, group, &held);
        for code in &keys {
            if let Some(c) = typed_char(state.key_get_utf32(xkb_code(*code))) {
                table.entry(c).or_insert(Key {
                    code: *code,
                    shift,
                    level3,
                });
            }
        }
    }
    table
}

fn typing_keys() -> Vec<u16> {
    crate::input::device_keys()
        .filter(|key: &KeyCode| !format!("{key:?}").starts_with("KEY_KP"))
        .map(|key: KeyCode| key.code())
        .collect()
}

fn pressed(keymap: &xkb::Keymap, group: u32, held: &[u16]) -> xkb::State {
    let mut state: xkb::State = xkb::State::new(keymap);
    state.update_mask(0, 0, 0, 0, 0, group);
    for code in held {
        state.update_key(xkb_code(*code), xkb::KeyDirection::Down);
    }
    state
}

fn key_for(state: &xkb::State, keys: &[u16], keysym: u32) -> Option<u16> {
    keys.iter()
        .copied()
        .find(|code: &u16| state.key_get_one_sym(xkb_code(*code)).raw() == keysym)
}

fn typed_char(utf32: u32) -> Option<char> {
    match char::from_u32(utf32) {
        Some('\r') => Some('\n'),
        Some('\t') => Some('\t'),
        Some(c) if c.is_control() => None,
        Some(c) => Some(c),
        None => None,
    }
}

fn xkb_code(code: u16) -> xkb::Keycode {
    xkb::Keycode::new(u32::from(code) + EVDEV_OFFSET)
}

#[cfg(test)]
mod tests;
