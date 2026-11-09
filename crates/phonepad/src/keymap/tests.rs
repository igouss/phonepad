use super::*;

fn layout(layout: &str, group: u32) -> KeyboardLayout {
    KeyboardLayout {
        rules: String::new(),
        model: String::new(),
        layout: String::from(layout),
        variant: String::new(),
        options: String::new(),
        active_layout_index: group,
    }
}

fn plain(code: u16) -> Option<Key> {
    Some(Key {
        code,
        shift: None,
        level3: None,
    })
}

fn shifted(code: u16) -> Option<Key> {
    Some(Key {
        code,
        shift: Some(42),
        level3: None,
    })
}

fn altgr(code: u16) -> Option<Key> {
    Some(Key {
        code,
        shift: None,
        level3: Some(100),
    })
}

fn lookup(name: &str, group: u32, c: char) -> Result<Option<Key>, KeymapError> {
    Ok(compile(&layout(name, group))?.get(&c).copied())
}

#[test]
fn us_letter_is_its_plain_key() -> Result<(), KeymapError> {
    assert_eq!(lookup("us", 0, 'z')?, plain(44));
    Ok(())
}

#[test]
fn us_capital_is_shift_plus_its_key() -> Result<(), KeymapError> {
    assert_eq!(lookup("us", 0, 'A')?, shifted(30));
    Ok(())
}

#[test]
fn us_at_sign_is_shift_2() -> Result<(), KeymapError> {
    assert_eq!(lookup("us", 0, '@')?, shifted(3));
    Ok(())
}

#[test]
fn us_less_than_is_the_plain_102nd_key_not_shift_comma() -> Result<(), KeymapError> {
    assert_eq!(lookup("us", 0, '<')?, plain(86));
    Ok(())
}

#[test]
fn newline_is_the_return_key_not_keypad_enter() -> Result<(), KeymapError> {
    assert_eq!(lookup("us", 0, '\n')?, plain(28));
    Ok(())
}

#[test]
fn carriage_return_has_no_key() -> Result<(), KeymapError> {
    assert_eq!(lookup("us", 0, '\r')?, None);
    Ok(())
}

#[test]
fn tab_is_the_tab_key() -> Result<(), KeymapError> {
    assert_eq!(lookup("us", 0, '\t')?, plain(15));
    Ok(())
}

#[test]
fn backspace_control_char_has_no_key() -> Result<(), KeymapError> {
    assert_eq!(lookup("us", 0, '\u{8}')?, None);
    Ok(())
}

#[test]
fn us_slash_is_the_main_key_not_the_keypad() -> Result<(), KeymapError> {
    assert_eq!(lookup("us", 0, '/')?, plain(53));
    Ok(())
}

#[test]
fn us_digit_is_the_number_row_not_shift_keypad() -> Result<(), KeymapError> {
    assert_eq!(lookup("us", 0, '1')?, plain(2));
    Ok(())
}

#[test]
fn us_emoji_has_no_key() -> Result<(), KeymapError> {
    assert_eq!(lookup("us", 0, '😀')?, None);
    Ok(())
}

#[test]
fn us_umlaut_has_no_key() -> Result<(), KeymapError> {
    assert_eq!(lookup("us", 0, 'ü')?, None);
    Ok(())
}

#[test]
fn de_z_is_the_us_y_key() -> Result<(), KeymapError> {
    assert_eq!(lookup("de", 0, 'z')?, plain(21));
    Ok(())
}

#[test]
fn de_y_is_the_us_z_key() -> Result<(), KeymapError> {
    assert_eq!(lookup("de", 0, 'y')?, plain(44));
    Ok(())
}

#[test]
fn de_at_sign_is_altgr_q() -> Result<(), KeymapError> {
    assert_eq!(lookup("de", 0, '@')?, altgr(16));
    Ok(())
}

#[test]
fn de_umlaut_is_a_plain_key() -> Result<(), KeymapError> {
    assert_eq!(lookup("de", 0, 'ü')?, plain(26));
    Ok(())
}

#[test]
fn de_capital_umlaut_is_shift_plus_its_key() -> Result<(), KeymapError> {
    assert_eq!(lookup("de", 0, 'Ü')?, shifted(26));
    Ok(())
}

#[test]
fn de_slash_is_shift_7_not_the_plain_keypad_key() -> Result<(), KeymapError> {
    assert_eq!(lookup("de", 0, '/')?, shifted(8));
    Ok(())
}

#[test]
fn de_capital_sharp_s_needs_shift_and_altgr() -> Result<(), KeymapError> {
    assert_eq!(
        lookup("de", 0, 'ẞ')?,
        Some(Key {
            code: 31,
            shift: Some(42),
            level3: Some(100),
        })
    );
    Ok(())
}

#[test]
fn de_dead_key_only_char_has_no_key() -> Result<(), KeymapError> {
    assert_eq!(lookup("de", 0, 'é')?, None);
    Ok(())
}

#[test]
fn de_emoji_has_no_key() -> Result<(), KeymapError> {
    assert_eq!(lookup("de", 0, '😀')?, None);
    Ok(())
}

#[test]
fn first_group_of_us_de_is_us() -> Result<(), KeymapError> {
    assert_eq!(lookup("us,de", 0, 'z')?, plain(44));
    Ok(())
}

#[test]
fn second_group_of_us_de_is_de() -> Result<(), KeymapError> {
    assert_eq!(lookup("us,de", 1, 'z')?, plain(21));
    Ok(())
}

#[test]
fn first_group_of_us_de_types_at_sign_as_shift_2() -> Result<(), KeymapError> {
    assert_eq!(lookup("us,de", 0, '@')?, shifted(3));
    Ok(())
}

#[test]
fn second_group_of_us_de_types_at_sign_as_altgr_q() -> Result<(), KeymapError> {
    assert_eq!(lookup("us,de", 1, '@')?, altgr(16));
    Ok(())
}

#[test]
fn group_past_the_last_layout_is_an_error() {
    assert_eq!(
        compile(&layout("us,de", 2)),
        Err(KeymapError::NoGroup {
            active: 2,
            count: 2,
        })
    );
}

#[test]
fn nul_in_a_name_is_an_error() {
    assert_eq!(compile(&layout("us\0", 0)), Err(KeymapError::Nul));
}

#[test]
fn unknown_layout_is_a_compile_error() {
    assert_eq!(
        compile(&layout("nosuchlayout", 0)),
        Err(KeymapError::Compile)
    );
}

#[test]
fn cache_compiles_a_switched_group() -> Result<(), KeymapError> {
    let mut keymaps: Keymaps = Keymaps::default();
    let first: Option<Key> = keymaps.table(layout("us,de", 0))?.get(&'z').copied();
    let second: Option<Key> = keymaps.table(layout("us,de", 1))?.get(&'z').copied();
    assert_eq!((first, second), (plain(44), plain(21)));
    Ok(())
}

#[test]
fn cache_returns_the_same_table_for_the_same_layout() -> Result<(), KeymapError> {
    let mut keymaps: Keymaps = Keymaps::default();
    let first: Table = keymaps.table(layout("de", 0))?.clone();
    let second: Table = keymaps.table(layout("de", 0))?.clone();
    assert_eq!(first, second);
    Ok(())
}
