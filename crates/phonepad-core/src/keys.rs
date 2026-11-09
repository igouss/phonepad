#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stroke {
    Key { code: u16, shift: bool },
    Unicode(String),
}

const KEY_1: u16 = 2;
const KEY_2: u16 = 3;
const KEY_3: u16 = 4;
const KEY_4: u16 = 5;
const KEY_5: u16 = 6;
const KEY_6: u16 = 7;
const KEY_7: u16 = 8;
const KEY_8: u16 = 9;
const KEY_9: u16 = 10;
const KEY_0: u16 = 11;
const KEY_MINUS: u16 = 12;
const KEY_EQUAL: u16 = 13;
const KEY_TAB: u16 = 15;
const KEY_Q: u16 = 16;
const KEY_W: u16 = 17;
const KEY_E: u16 = 18;
const KEY_R: u16 = 19;
const KEY_T: u16 = 20;
const KEY_Y: u16 = 21;
const KEY_U: u16 = 22;
const KEY_I: u16 = 23;
const KEY_O: u16 = 24;
const KEY_P: u16 = 25;
const KEY_LEFTBRACE: u16 = 26;
const KEY_RIGHTBRACE: u16 = 27;
const KEY_ENTER: u16 = 28;
const KEY_A: u16 = 30;
const KEY_S: u16 = 31;
const KEY_D: u16 = 32;
const KEY_F: u16 = 33;
const KEY_G: u16 = 34;
const KEY_H: u16 = 35;
const KEY_J: u16 = 36;
const KEY_K: u16 = 37;
const KEY_L: u16 = 38;
const KEY_SEMICOLON: u16 = 39;
const KEY_APOSTROPHE: u16 = 40;
const KEY_GRAVE: u16 = 41;
const KEY_BACKSLASH: u16 = 43;
const KEY_Z: u16 = 44;
const KEY_X: u16 = 45;
const KEY_C: u16 = 46;
const KEY_V: u16 = 47;
const KEY_B: u16 = 48;
const KEY_N: u16 = 49;
const KEY_M: u16 = 50;
const KEY_COMMA: u16 = 51;
const KEY_DOT: u16 = 52;
const KEY_SLASH: u16 = 53;
const KEY_SPACE: u16 = 57;

fn key(c: char) -> Option<(u16, bool)> {
    let unshifted: u16 = match c {
        'a' | 'A' => KEY_A,
        'b' | 'B' => KEY_B,
        'c' | 'C' => KEY_C,
        'd' | 'D' => KEY_D,
        'e' | 'E' => KEY_E,
        'f' | 'F' => KEY_F,
        'g' | 'G' => KEY_G,
        'h' | 'H' => KEY_H,
        'i' | 'I' => KEY_I,
        'j' | 'J' => KEY_J,
        'k' | 'K' => KEY_K,
        'l' | 'L' => KEY_L,
        'm' | 'M' => KEY_M,
        'n' | 'N' => KEY_N,
        'o' | 'O' => KEY_O,
        'p' | 'P' => KEY_P,
        'q' | 'Q' => KEY_Q,
        'r' | 'R' => KEY_R,
        's' | 'S' => KEY_S,
        't' | 'T' => KEY_T,
        'u' | 'U' => KEY_U,
        'v' | 'V' => KEY_V,
        'w' | 'W' => KEY_W,
        'x' | 'X' => KEY_X,
        'y' | 'Y' => KEY_Y,
        'z' | 'Z' => KEY_Z,
        '`' | '~' => KEY_GRAVE,
        '1' | '!' => KEY_1,
        '2' | '@' => KEY_2,
        '3' | '#' => KEY_3,
        '4' | '$' => KEY_4,
        '5' | '%' => KEY_5,
        '6' | '^' => KEY_6,
        '7' | '&' => KEY_7,
        '8' | '*' => KEY_8,
        '9' | '(' => KEY_9,
        '0' | ')' => KEY_0,
        '-' | '_' => KEY_MINUS,
        '=' | '+' => KEY_EQUAL,
        '[' | '{' => KEY_LEFTBRACE,
        ']' | '}' => KEY_RIGHTBRACE,
        '\\' | '|' => KEY_BACKSLASH,
        ';' | ':' => KEY_SEMICOLON,
        '\'' | '"' => KEY_APOSTROPHE,
        ',' | '<' => KEY_COMMA,
        '.' | '>' => KEY_DOT,
        '/' | '?' => KEY_SLASH,
        ' ' => KEY_SPACE,
        '\t' => KEY_TAB,
        '\n' => KEY_ENTER,
        _ => return None,
    };
    Some((
        unshifted,
        c.is_ascii_uppercase() || "~!@#$%^&*()_+{}|:\"<>?".contains(c),
    ))
}

pub fn typing_plan(text: &str) -> Vec<Stroke> {
    text.chars()
        .fold(Vec::new(), |mut plan: Vec<Stroke>, c: char| {
            match key(c) {
                Some((code, shift)) => plan.push(Stroke::Key { code, shift }),
                None => match plan.last_mut() {
                    Some(Stroke::Unicode(run)) => run.push(c),
                    Some(Stroke::Key { .. }) | None => plan.push(Stroke::Unicode(c.to_string())),
                },
            }
            plan
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(code: u16) -> Stroke {
        Stroke::Key { code, shift: false }
    }

    fn shifted(code: u16) -> Stroke {
        Stroke::Key { code, shift: true }
    }

    fn unicode(text: &str) -> Stroke {
        Stroke::Unicode(text.to_string())
    }

    #[test]
    fn empty_text_types_nothing() {
        assert_eq!(typing_plan(""), Vec::<Stroke>::new());
    }

    #[test]
    fn one_letter_is_one_key() {
        assert_eq!(typing_plan("a"), vec![plain(30)]);
    }

    #[test]
    fn two_letters_are_two_keys() {
        assert_eq!(typing_plan("ab"), vec![plain(30), plain(48)]);
    }

    #[test]
    fn lowercase_letters_map_to_their_keys() {
        assert_eq!(
            typing_plan("abcdefghijklmnopqrstuvwxyz"),
            vec![
                plain(30),
                plain(48),
                plain(46),
                plain(32),
                plain(18),
                plain(33),
                plain(34),
                plain(35),
                plain(23),
                plain(36),
                plain(37),
                plain(38),
                plain(50),
                plain(49),
                plain(24),
                plain(25),
                plain(16),
                plain(19),
                plain(31),
                plain(20),
                plain(22),
                plain(47),
                plain(17),
                plain(45),
                plain(21),
                plain(44),
            ]
        );
    }

    #[test]
    fn uppercase_letters_map_to_shifted_keys() {
        assert_eq!(
            typing_plan("ABCDEFGHIJKLMNOPQRSTUVWXYZ"),
            vec![
                shifted(30),
                shifted(48),
                shifted(46),
                shifted(32),
                shifted(18),
                shifted(33),
                shifted(34),
                shifted(35),
                shifted(23),
                shifted(36),
                shifted(37),
                shifted(38),
                shifted(50),
                shifted(49),
                shifted(24),
                shifted(25),
                shifted(16),
                shifted(19),
                shifted(31),
                shifted(20),
                shifted(22),
                shifted(47),
                shifted(17),
                shifted(45),
                shifted(21),
                shifted(44),
            ]
        );
    }

    #[test]
    fn unshifted_symbols_map_to_their_keys() {
        assert_eq!(
            typing_plan("`1234567890-=[]\\;',./"),
            vec![
                plain(41),
                plain(2),
                plain(3),
                plain(4),
                plain(5),
                plain(6),
                plain(7),
                plain(8),
                plain(9),
                plain(10),
                plain(11),
                plain(12),
                plain(13),
                plain(26),
                plain(27),
                plain(43),
                plain(39),
                plain(40),
                plain(51),
                plain(52),
                plain(53),
            ]
        );
    }

    #[test]
    fn shifted_symbols_map_to_shifted_keys() {
        assert_eq!(
            typing_plan("~!@#$%^&*()_+{}|:\"<>?"),
            vec![
                shifted(41),
                shifted(2),
                shifted(3),
                shifted(4),
                shifted(5),
                shifted(6),
                shifted(7),
                shifted(8),
                shifted(9),
                shifted(10),
                shifted(11),
                shifted(12),
                shifted(13),
                shifted(26),
                shifted(27),
                shifted(43),
                shifted(39),
                shifted(40),
                shifted(51),
                shifted(52),
                shifted(53),
            ]
        );
    }

    #[test]
    fn whitespace_maps_to_space_tab_enter() {
        assert_eq!(typing_plan(" \t\n"), vec![plain(57), plain(15), plain(28)]);
    }

    #[test]
    fn one_unmapped_char_is_one_unicode_stroke() {
        assert_eq!(typing_plan("é"), vec![unicode("é")]);
    }

    #[test]
    fn unmapped_run_merges_into_one_unicode_stroke() {
        assert_eq!(typing_plan("é\r€"), vec![unicode("é\r€")]);
    }

    #[test]
    fn mapped_key_splits_unicode_runs() {
        assert_eq!(
            typing_plan("éü a€"),
            vec![unicode("éü"), plain(57), plain(30), unicode("€")]
        );
    }
}
