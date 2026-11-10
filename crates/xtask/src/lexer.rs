struct Scanner {
    chars: Vec<char>,
    pos: usize,
    end: usize,
    line: usize,
    hits: Vec<usize>,
    ambiguous: Vec<usize>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Scan {
    pub comments: Vec<usize>,
    pub ambiguous_slashes: Vec<usize>,
}

impl Scanner {
    fn new(src: &str) -> Scanner {
        let chars: Vec<char> = src.chars().collect();
        let end: usize = chars.len();
        Scanner {
            chars,
            pos: 0,
            end,
            line: 1,
            hits: Vec::new(),
            ambiguous: Vec::new(),
        }
    }

    fn at(&self, offset: usize) -> Option<char> {
        let index: usize = self.pos + offset;
        if index < self.end {
            self.chars.get(index).copied()
        } else {
            None
        }
    }

    fn starts_with(&self, pattern: &str) -> bool {
        pattern
            .chars()
            .enumerate()
            .all(|(offset, expected)| self.at(offset) == Some(expected))
    }

    fn starts_with_ignore_case(&self, pattern: &str) -> bool {
        pattern.chars().enumerate().all(|(offset, expected)| {
            self.at(offset)
                .is_some_and(|c| c.eq_ignore_ascii_case(&expected))
        })
    }

    fn done(&self) -> bool {
        self.at(0).is_none()
    }

    fn bump(&mut self) {
        if let Some(c) = self.at(0) {
            if c == '\n' {
                self.line += 1;
            }
            self.pos += 1;
        }
    }

    fn bump_n(&mut self, count: usize) {
        (0..count).for_each(|_| self.bump());
    }

    fn line_comment(&mut self) {
        self.hits.push(self.line);
        while self.at(0).is_some_and(|c| c != '\n') {
            self.bump();
        }
    }

    fn block_comment(&mut self, nests: bool) {
        self.hits.push(self.line);
        self.bump_n(2);
        let mut depth: usize = 1;
        while depth > 0 && !self.done() {
            if self.starts_with("*/") {
                self.bump_n(2);
                depth -= 1;
            } else if nests && self.starts_with("/*") {
                self.bump_n(2);
                depth += 1;
            } else {
                self.bump();
            }
        }
    }

    fn quoted(&mut self, quote: char) {
        self.bump();
        while let Some(c) = self.at(0) {
            if c == '\\' {
                self.bump_n(2);
            } else {
                self.bump();
                if c == quote {
                    return;
                }
            }
        }
    }

    fn identifier(&mut self) -> String {
        let mut word: String = String::new();
        while let Some(c) = self
            .at(0)
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
        {
            word.push(c);
            self.bump();
        }
        word
    }

    fn find_ignore_case(&mut self, pattern: &str) -> usize {
        let start: usize = self.pos;
        let start_line: usize = self.line;
        while !self.done() && !self.starts_with_ignore_case(pattern) {
            self.bump();
        }
        let found: usize = self.pos;
        self.pos = start;
        self.line = start_line;
        found
    }
}

fn starts_identifier(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

pub fn rust(src: &str) -> Vec<usize> {
    let mut s: Scanner = Scanner::new(src);
    if s.starts_with("#!") && !s.starts_with("#![") {
        while s.at(0).is_some_and(|c| c != '\n') {
            s.bump();
        }
    }
    while let Some(c) = s.at(0) {
        if s.starts_with("//") {
            s.line_comment();
        } else if s.starts_with("/*") {
            s.block_comment(true);
        } else if c == '"' {
            s.quoted('"');
        } else if c == '\'' {
            rust_char_or_lifetime(&mut s);
        } else if starts_identifier(c) {
            let word: String = s.identifier();
            if matches!(word.as_str(), "r" | "br" | "cr") {
                rust_raw_string(&mut s);
            }
        } else {
            s.bump();
        }
    }
    s.hits
}

fn rust_char_or_lifetime(s: &mut Scanner) {
    if s.at(1) == Some('\\') {
        s.bump_n(3);
        while s.at(0).is_some_and(|c| c != '\'') {
            s.bump();
        }
        s.bump();
    } else if s.at(2) == Some('\'') {
        s.bump_n(3);
    } else {
        s.bump();
    }
}

fn rust_raw_string(s: &mut Scanner) {
    let mut hashes: usize = 0;
    while s.at(hashes) == Some('#') {
        hashes += 1;
    }
    if s.at(hashes) != Some('"') {
        return;
    }
    s.bump_n(hashes + 1);
    let closing: String = std::iter::once('"')
        .chain(std::iter::repeat_n('#', hashes))
        .collect();
    while !s.done() && !s.starts_with(&closing) {
        s.bump();
    }
    s.bump_n(closing.chars().count());
}

pub fn html(src: &str) -> Scan {
    let mut s: Scanner = Scanner::new(src);
    while let Some(c) = s.at(0) {
        if s.starts_with("<!--") {
            s.hits.push(s.line);
            while !s.done() && !s.starts_with("-->") {
                s.bump();
            }
            s.bump_n(3);
        } else if c == '<' && s.at(1).is_some_and(|n| n.is_ascii_alphabetic()) {
            s.bump();
            let tag: String = s.identifier().to_ascii_lowercase();
            html_tag_rest(&mut s);
            match tag.as_str() {
                "script" => embedded(&mut s, "</script", js_until_end),
                "style" => embedded(&mut s, "</style", css),
                _ => {}
            }
        } else {
            s.bump();
        }
    }
    Scan {
        comments: s.hits,
        ambiguous_slashes: s.ambiguous,
    }
}

pub fn javascript(src: &str) -> Scan {
    let mut s: Scanner = Scanner::new(src);
    js(&mut s, false);
    Scan {
        comments: s.hits,
        ambiguous_slashes: s.ambiguous,
    }
}

fn html_tag_rest(s: &mut Scanner) {
    while let Some(c) = s.at(0) {
        if c == '"' || c == '\'' {
            s.quoted(c);
        } else {
            s.bump();
            if c == '>' {
                return;
            }
        }
    }
}

fn embedded(s: &mut Scanner, closing: &str, lex: fn(&mut Scanner)) {
    let outer_end: usize = s.end;
    s.end = s.find_ignore_case(closing);
    lex(s);
    s.end = outer_end;
}

fn css(s: &mut Scanner) {
    while let Some(c) = s.at(0) {
        if s.starts_with("/*") {
            s.block_comment(false);
        } else if c == '"' || c == '\'' {
            s.quoted(c);
        } else {
            s.bump();
        }
    }
}

fn js_until_end(s: &mut Scanner) {
    js(s, false);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SlashMeans {
    Division,
    Regex,
    Unknown,
}

const EXPRESSION_KEYWORDS: [&str; 15] = [
    "return",
    "typeof",
    "instanceof",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "throw",
    "case",
    "do",
    "else",
    "yield",
    "await",
    "extends",
];

const CONDITION_KEYWORDS: [&str; 4] = ["if", "while", "for", "with"];

fn js(s: &mut Scanner, until_closing_brace: bool) {
    let mut depth: usize = 0;
    let mut slash: SlashMeans = SlashMeans::Regex;
    let mut word: String = String::new();
    let mut parens: Vec<bool> = Vec::new();
    while let Some(c) = s.at(0) {
        let previous_word: String = std::mem::take(&mut word);
        if c.is_whitespace() {
            word = previous_word;
            s.bump();
        } else if s.starts_with("//") {
            word = previous_word;
            s.line_comment();
        } else if s.starts_with("/*") {
            word = previous_word;
            s.block_comment(false);
        } else if c == '/' {
            match slash {
                SlashMeans::Division => s.bump(),
                SlashMeans::Regex => regex(s),
                SlashMeans::Unknown => {
                    s.ambiguous.push(s.line);
                    s.bump();
                }
            }
            slash = if slash == SlashMeans::Regex {
                SlashMeans::Division
            } else {
                SlashMeans::Regex
            };
        } else if c == '"' || c == '\'' {
            s.quoted(c);
            slash = SlashMeans::Division;
        } else if c == '`' {
            template(s);
            slash = SlashMeans::Division;
        } else if c.is_alphabetic() || c == '_' || c == '$' {
            word = s.identifier();
            slash = if EXPRESSION_KEYWORDS.contains(&word.as_str()) {
                SlashMeans::Regex
            } else {
                SlashMeans::Division
            };
        } else if c.is_ascii_digit() {
            while s.at(0).is_some_and(|d| d.is_alphanumeric() || d == '.') {
                s.bump();
            }
            slash = SlashMeans::Division;
        } else if s.starts_with("++") || s.starts_with("--") {
            s.bump_n(2);
            slash = SlashMeans::Unknown;
        } else if c == '(' {
            parens.push(CONDITION_KEYWORDS.contains(&previous_word.as_str()));
            s.bump();
            slash = SlashMeans::Regex;
        } else if c == ')' {
            let condition: bool = parens.pop().unwrap_or(false);
            s.bump();
            slash = if condition {
                SlashMeans::Regex
            } else {
                SlashMeans::Division
            };
        } else if c == ']' {
            s.bump();
            slash = SlashMeans::Division;
        } else if c == '{' {
            depth += 1;
            s.bump();
            slash = SlashMeans::Regex;
        } else if c == '}' {
            s.bump();
            if depth == 0 && until_closing_brace {
                return;
            }
            depth = depth.saturating_sub(1);
            slash = SlashMeans::Unknown;
        } else {
            s.bump();
            slash = SlashMeans::Regex;
        }
    }
}

fn regex(s: &mut Scanner) {
    s.bump();
    let mut in_class: bool = false;
    while let Some(c) = s.at(0) {
        if c == '\n' {
            return;
        } else if c == '\\' {
            s.bump_n(2);
        } else {
            s.bump();
            if c == '[' {
                in_class = true;
            } else if c == ']' {
                in_class = false;
            } else if c == '/' && !in_class {
                s.identifier();
                return;
            }
        }
    }
}

fn template(s: &mut Scanner) {
    s.bump();
    while let Some(c) = s.at(0) {
        if c == '\\' {
            s.bump_n(2);
        } else if s.starts_with("${") {
            s.bump_n(2);
            js(s, true);
        } else {
            s.bump();
            if c == '`' {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Scan, html, javascript, rust};

    fn comments(src: &str) -> Vec<usize> {
        html(src).comments
    }

    #[test]
    fn rust_without_comments_has_no_hits() {
        assert_eq!(rust("fn main() {}\n"), Vec::<usize>::new());
    }

    #[test]
    fn rust_line_comment_is_reported_at_its_line() {
        assert_eq!(rust("fn a() {}\n// note\n"), vec![2]);
    }

    #[test]
    fn rust_reports_every_comment_kind() {
        let src: &str = "/// doc\n//! inner\nfn a() {} /* b */\n/* outer /* nested */\n still */ fn c() {}\n// last\n";
        assert_eq!(rust(src), vec![1, 2, 3, 4, 6]);
    }

    #[test]
    fn rust_comment_markers_in_strings_are_not_comments() {
        let src: &str = "let a = \"// not \\\" /* a comment\";\nlet b = \"x\";\n";
        assert_eq!(rust(src), Vec::<usize>::new());
    }

    #[test]
    fn rust_raw_strings_hide_markers_and_quotes() {
        let src: &str = "let a = r\"//\";\nlet b = r#\"\" // \"#;\nlet c = br##\"\"# /*\"##;\nlet d = cr\"//\";\n";
        assert_eq!(rust(src), Vec::<usize>::new());
    }

    #[test]
    fn rust_byte_and_c_strings_hide_markers() {
        assert_eq!(
            rust("let a = b\"// x\";\nlet b = c\"/* y\";\n"),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn rust_raw_identifier_is_not_a_raw_string() {
        assert_eq!(rust("let r#type = 1; // here\n"), vec![1]);
    }

    #[test]
    fn rust_char_literals_hide_markers() {
        let src: &str = "let a = '/'; let b = '\"'; let c = '\\''; let d = b'/'; let e = '\\u{2F}';\n// after\n";
        assert_eq!(rust(src), vec![2]);
    }

    #[test]
    fn rust_lifetimes_are_not_char_literals() {
        let src: &str = "fn f<'a>(x: &'a str) -> &'static str where 'a: '_ { x }\nconst S: &str = \"'\"; // hit\n";
        assert_eq!(rust(src), vec![2]);
    }

    #[test]
    fn rust_attributes_and_shebang_are_not_comments() {
        assert_eq!(
            rust(
                "#!/usr/bin/env run-cargo-script\n#![forbid(unsafe_code)]\n#[derive(Debug)]\nstruct A;\n"
            ),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn rust_inner_attribute_on_line_one_is_not_a_shebang() {
        assert_eq!(rust("#![allow(dead_code)] // hit\n"), vec![1]);
    }

    #[test]
    fn javascript_without_comments_has_no_hits() {
        assert_eq!(
            javascript("const a = \"// no\", b = 4 / 2;\n"),
            Scan {
                comments: Vec::new(),
                ambiguous_slashes: Vec::new()
            }
        );
    }

    #[test]
    fn javascript_reports_line_and_block_comments() {
        assert_eq!(
            javascript("let a = 1;\n// one\n/* two */ let b = 2;\n").comments,
            vec![2, 3]
        );
    }

    #[test]
    fn javascript_reports_an_ambiguous_slash() {
        assert_eq!(javascript("a++\n/ 2;\n").ambiguous_slashes, vec![2]);
    }

    #[test]
    fn html_without_comments_has_no_hits() {
        assert_eq!(comments("<p>hi</p>\n"), Vec::<usize>::new());
    }

    #[test]
    fn html_markup_comment_is_reported() {
        assert_eq!(comments("<p>hi</p>\n<!-- note -->\n"), vec![2]);
    }

    #[test]
    fn html_reports_css_and_js_comments() {
        let src: &str = "<style>\na { b: c; } /* css */\n</style>\n<script>\nlet a = 1; // js\n/* block */\n</script>\n<!-- html -->\n";
        assert_eq!(comments(src), vec![2, 5, 6, 8]);
    }

    #[test]
    fn css_double_slash_and_strings_are_not_comments() {
        let src: &str =
            "<style>\na { background: url(http://x/y); content: \"/* no */\"; }\n</style>\n";
        assert_eq!(comments(src), Vec::<usize>::new());
    }

    #[test]
    fn js_strings_and_division_are_not_comments() {
        let src: &str =
            "<script>\nconst a = \"// no\", b = '/* no', c = 4 / 2;\nd /= 2;\n</script>\n";
        assert_eq!(comments(src), Vec::<usize>::new());
    }

    #[test]
    fn js_template_literals_with_nested_code_are_not_comments() {
        let src: &str =
            "<script>\nconst u = `${a ? \"wss\" : `x${{b: 1}.b}`}://${h}/ws`; // hit\n</script>\n";
        assert_eq!(comments(src), vec![2]);
    }

    #[test]
    fn js_comment_inside_template_substitution_is_reported() {
        let src: &str = "<script>\nconst u = `${a /* hit */}`;\n</script>\n";
        assert_eq!(comments(src), vec![2]);
    }

    #[test]
    fn html_attribute_values_are_not_comments() {
        assert_eq!(
            comments("<a href=\"http://x\" title='a > b <!-- no -->'>x</a>\n"),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn js_lexing_stops_at_the_closing_script_tag() {
        let src: &str = "<script>\nconst a = \"</script>\n<p>// text</p>\n";
        assert_eq!(comments(src), Vec::<usize>::new());
    }

    #[test]
    fn rust_nested_block_comment_is_one_comment() {
        assert_eq!(rust("/* a /* b */ // c */ fn d() {}\n"), vec![1]);
    }

    #[test]
    fn rust_unterminated_block_comment_is_reported_and_ends_at_eof() {
        assert_eq!(rust("fn a() {}\n/* open"), vec![2]);
    }

    #[test]
    fn rust_unterminated_raw_string_ends_at_eof() {
        assert_eq!(rust("let a = r#\"open // x\n"), Vec::<usize>::new());
    }

    #[test]
    fn rust_shebang_line_is_skipped_even_with_slashes() {
        assert_eq!(
            rust("#!/usr/bin/env -S run //script\nfn a() {} // b\n"),
            vec![2]
        );
    }

    #[test]
    fn rust_escaped_char_literal_ends_at_its_quote() {
        assert_eq!(rust("let c = '\\t'// x\n"), vec![1]);
    }

    #[test]
    fn html_unterminated_comment_ends_at_eof() {
        assert_eq!(comments("<p>\n<!-- open"), vec![2]);
    }

    #[test]
    fn html_comment_hides_markup_inside_it() {
        assert_eq!(comments("<!-- <script>// x</script> -->\n"), vec![1]);
    }

    #[test]
    fn a_word_after_a_space_is_not_a_tag() {
        assert_eq!(comments("<p>x script> // y</p>\n"), Vec::<usize>::new());
    }

    #[test]
    fn js_template_text_is_not_code() {
        assert_eq!(
            comments("<script>\nconst u = `a // b /* c`;\n</script>\n"),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn js_object_literal_inside_a_substitution_stays_code() {
        assert_eq!(
            comments("<script>\nconst u = `${ {a: 1}[\"a\"] /* c */ }`;\n</script>\n"),
            vec![2]
        );
    }

    #[test]
    fn js_regex_literals_hide_markers_and_quotes() {
        let src: &str = "<script>\nconst a = /\\/\\/'\"[/]/g; f(/\\*/); // hit\n</script>\n";
        assert_eq!(
            html(src),
            Scan {
                comments: vec![2],
                ambiguous_slashes: vec![]
            }
        );
    }

    #[test]
    fn js_regex_after_a_condition_is_a_regex() {
        let src: &str = "<script>\nif (x) /'/.test(y); // hit\n</script>\n";
        assert_eq!(
            html(src),
            Scan {
                comments: vec![2],
                ambiguous_slashes: vec![]
            }
        );
    }

    #[test]
    fn js_division_after_operands_is_not_ambiguous() {
        let src: &str = "<script>\nconst s = Math.min(W / w, (H - h) / 2, a[0] / 3, 4 / 2); d /= n;\n</script>\n";
        assert_eq!(
            html(src),
            Scan {
                comments: vec![],
                ambiguous_slashes: vec![]
            }
        );
    }

    #[test]
    fn js_slash_after_a_brace_or_increment_is_reported_ambiguous() {
        let src: &str = "<script>\nfunction f() {}\n/x/.test(a);\ni++ / 2;\n</script>\n";
        assert_eq!(
            html(src),
            Scan {
                comments: vec![],
                ambiguous_slashes: vec![3, 4]
            }
        );
    }

    #[test]
    fn js_slash_after_a_regex_is_division() {
        assert_eq!(
            comments("<script>\nx = /a/ / 2; // hit\n</script>\n"),
            vec![2]
        );
    }

    #[test]
    fn js_underscore_identifier_is_an_operand() {
        assert_eq!(
            comments("<script>\nx = _ / 2; // hit\n</script>\n"),
            vec![2]
        );
    }
}
