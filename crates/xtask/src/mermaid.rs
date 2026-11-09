use std::collections::{BTreeMap, BTreeSet};

pub fn sanitise_id(prefix: &str, label: &str) -> String {
    let mut id: String = String::from(prefix);
    for c in label.chars() {
        if c.is_ascii_alphanumeric() {
            id.push(c);
        } else if !id.ends_with('_') {
            id.push('_');
        }
    }
    id.trim_end_matches('_').to_string()
}

pub fn escape_label(label: &str) -> String {
    label
        .chars()
        .map(|c| match c {
            '"' => String::from("#quot;"),
            '#' => String::from("#35;"),
            ';' => String::from("#59;"),
            '<' => String::from("#lt;"),
            '>' => String::from("#gt;"),
            other => other.to_string(),
        })
        .collect()
}

#[derive(Default)]
pub struct Ids {
    by_label: BTreeMap<String, String>,
    taken: BTreeSet<String>,
}

impl Ids {
    pub fn id(&mut self, prefix: &str, label: &str) -> String {
        if let Some(existing) = self.by_label.get(label) {
            return existing.clone();
        }
        let base: String = sanitise_id(prefix, label);
        let mut id: String = base.clone();
        let mut suffix: usize = 1;
        while self.taken.contains(&id) {
            suffix += 1;
            id = format!("{base}_{suffix}");
        }
        self.taken.insert(id.clone());
        self.by_label.insert(label.to_string(), id.clone());
        id
    }
}

#[cfg(test)]
mod tests {
    use super::{Ids, escape_label, sanitise_id};

    #[test]
    fn empty_label_yields_the_prefix() {
        assert_eq!(sanitise_id("s_", ""), "s");
    }

    #[test]
    fn plain_label_is_kept() {
        assert_eq!(sanitise_id("s_", "Hidden"), "s_Hidden");
    }

    #[test]
    fn runs_of_punctuation_collapse_to_one_underscore() {
        assert_eq!(
            sanitise_id("s_", "Streaming { in_flight: 0, frame_pending: false }"),
            "s_Streaming_in_flight_0_frame_pending_false"
        );
    }

    #[test]
    fn hyphens_and_dots_become_underscores() {
        assert_eq!(
            sanitise_id("c_", "phonepad-core 0.1.0"),
            "c_phonepad_core_0_1_0"
        );
    }

    #[test]
    fn non_ascii_is_replaced() {
        assert_eq!(sanitise_id("s_", "Größe"), "s_Gr_e");
    }

    #[test]
    fn escape_leaves_plain_text_alone() {
        assert_eq!(escape_label("Hidden"), "Hidden");
    }

    #[test]
    fn escape_encodes_mermaid_syntax() {
        assert_eq!(
            escape_label("A { x: \"#;\" } <b>"),
            "A { x: #quot;#35;#59;#quot; } #lt;b#gt;"
        );
    }

    #[test]
    fn ids_are_stable_per_label() {
        let mut ids: Ids = Ids::default();
        let first: String = ids.id("s_", "A");
        assert_eq!(ids.id("s_", "A"), first);
    }

    #[test]
    fn a_label_that_looks_like_a_suffixed_id_does_not_collide() {
        let mut ids: Ids = Ids::default();
        let a: String = ids.id("s_", "P_2");
        let b: String = ids.id("s_", "P");
        let c: String = ids.id("s_", "P!");
        assert_eq!(
            (a.as_str(), b.as_str(), c.as_str()),
            ("s_P_2", "s_P", "s_P_3")
        );
    }

    #[test]
    fn colliding_labels_get_distinct_ids() {
        let mut ids: Ids = Ids::default();
        let a: String = ids.id("s_", "P { x: -1 }");
        let b: String = ids.id("s_", "P { x: 1 }");
        let c: String = ids.id("s_", "P(x: 1)");
        assert_eq!(
            (a.as_str(), b.as_str(), c.as_str()),
            ("s_P_x_1", "s_P_x_1_2", "s_P_x_1_3")
        );
    }
}
