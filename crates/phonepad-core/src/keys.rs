use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub code: u16,
    pub shift: Option<u16>,
    pub level3: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stroke {
    Key(Key),
    Unicode(String),
}

pub type Table = BTreeMap<char, Key>;

pub fn typing_plan(text: &str, table: &Table) -> Vec<Stroke> {
    text.chars()
        .fold(Vec::new(), |mut plan: Vec<Stroke>, c: char| {
            match table.get(&c) {
                Some(key) => plan.push(Stroke::Key(*key)),
                None => match plan.last_mut() {
                    Some(Stroke::Unicode(run)) => run.push(c),
                    Some(Stroke::Key(_)) | None => plan.push(Stroke::Unicode(c.to_string())),
                },
            }
            plan
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Key = Key {
        code: 30,
        shift: None,
        level3: None,
    };
    const SHIFTED_A: Key = Key {
        code: 30,
        shift: Some(42),
        level3: None,
    };
    const AT: Key = Key {
        code: 16,
        shift: None,
        level3: Some(100),
    };
    const SPACE: Key = Key {
        code: 57,
        shift: None,
        level3: None,
    };

    fn table() -> Table {
        Table::from([('a', A), ('A', SHIFTED_A), ('@', AT), (' ', SPACE)])
    }

    fn unicode(text: &str) -> Stroke {
        Stroke::Unicode(text.to_string())
    }

    #[test]
    fn empty_text_types_nothing() {
        assert_eq!(typing_plan("", &table()), Vec::<Stroke>::new());
    }

    #[test]
    fn one_mapped_char_is_its_key() {
        assert_eq!(typing_plan("a", &table()), vec![Stroke::Key(A)]);
    }

    #[test]
    fn mapped_chars_are_one_key_each_with_their_modifiers() {
        assert_eq!(
            typing_plan("aA@", &table()),
            vec![Stroke::Key(A), Stroke::Key(SHIFTED_A), Stroke::Key(AT)]
        );
    }

    #[test]
    fn empty_table_types_everything_as_one_unicode_run() {
        assert_eq!(typing_plan("aA", &Table::new()), vec![unicode("aA")]);
    }

    #[test]
    fn one_unmapped_char_is_one_unicode_stroke() {
        assert_eq!(typing_plan("é", &table()), vec![unicode("é")]);
    }

    #[test]
    fn unmapped_run_merges_into_one_unicode_stroke() {
        assert_eq!(typing_plan("é\r€", &table()), vec![unicode("é\r€")]);
    }

    #[test]
    fn mapped_key_splits_unicode_runs() {
        assert_eq!(
            typing_plan("éü a€", &table()),
            vec![
                unicode("éü"),
                Stroke::Key(SPACE),
                Stroke::Key(A),
                unicode("€")
            ]
        );
    }
}
