use crate::lexer::{self, Scan};
use crate::profile::HOT_PATH;
use crate::views::{Files, VIEWS_DIR, read_dir_files, regenerate};
use crate::workspace::{self, Member, Workspace};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub const CORE: &str = "phonepad-core";
pub const HTML_FILES: [&str; 1] = ["pad.html"];

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail(Vec<String>),
}

impl From<Result<Vec<String>, String>> for Verdict {
    fn from(result: Result<Vec<String>, String>) -> Verdict {
        match result {
            Ok(findings) if findings.is_empty() => Verdict::Pass,
            Ok(findings) => Verdict::Fail(findings),
            Err(error) => Verdict::Fail(vec![format!("error: {error}")]),
        }
    }
}

pub fn all(root: &Path) -> Vec<(&'static str, Verdict)> {
    vec![
        (
            "core-has-no-dependencies",
            core_has_no_dependencies(root).into(),
        ),
        ("no-comments", no_comments(root).into()),
        ("views-fresh", views_fresh(root).into()),
    ]
}

fn core_has_no_dependencies(root: &Path) -> Result<Vec<String>, String> {
    core_findings(&workspace::load(root)?)
}

fn core_findings(workspace: &Workspace) -> Result<Vec<String>, String> {
    let core: &Member = workspace
        .member(CORE)
        .ok_or_else(|| format!("{CORE} is not a workspace member"))?;
    Ok(core
        .dependencies
        .iter()
        .map(|p| {
            format!(
                "{CORE} depends on {} {} ({})",
                p.name, p.version, core.manifest
            )
        })
        .collect())
}

fn tracked_files(root: &Path) -> Result<Vec<String>, String> {
    let output: Output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let listing: String =
        String::from_utf8(output.stdout).map_err(|e| format!("git ls-files: {e}"))?;
    Ok(listing
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect())
}

#[derive(Clone, Copy)]
enum Language {
    Rust,
    Html,
}

fn language(path: &str) -> Option<Language> {
    if path.ends_with(".rs") {
        Some(Language::Rust)
    } else if HTML_FILES.contains(&path) {
        Some(Language::Html)
    } else {
        None
    }
}

fn comment_findings(path: &str, language: Language, source: &str) -> Vec<String> {
    let scan: Scan = match language {
        Language::Rust => Scan {
            comments: lexer::rust(source),
            ambiguous_slashes: Vec::new(),
        },
        Language::Html => lexer::html(source),
    };
    let mut findings: Vec<String> = scan
        .comments
        .into_iter()
        .map(|line| format!("{path}:{line}"))
        .collect();
    findings.extend(scan.ambiguous_slashes.into_iter().map(|line| {
        format!(
            "{path}:{line} `/` could start a regex or be a division; the lexer cannot classify it"
        )
    }));
    findings
}

fn no_comments(root: &Path) -> Result<Vec<String>, String> {
    let mut findings: Vec<String> = Vec::new();
    for path in tracked_files(root)? {
        let Some(language) = language(&path) else {
            continue;
        };
        let source: String = match std::fs::read_to_string(root.join(&path)) {
            Ok(source) => source,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{path}: {e}")),
        };
        findings.extend(comment_findings(&path, language, &source));
    }
    Ok(findings)
}

fn views_fresh(root: &Path) -> Result<Vec<String>, String> {
    let scratch: PathBuf = root.join("target/xtask/views-fresh");
    if scratch.exists() {
        std::fs::remove_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    }
    regenerate(root, &scratch)?;
    let fresh: Files = read_dir_files(&scratch)?;
    if fresh.is_empty() {
        return Err(String::from("regeneration produced no files"));
    }
    let views: PathBuf = root.join(VIEWS_DIR);
    let committed: Files = if views.exists() {
        read_dir_files(&views)?
    } else {
        Files::new()
    };
    Ok(stale_views(&fresh, &committed))
}

fn stale_views(fresh: &Files, committed: &Files) -> Vec<String> {
    let mut findings: Vec<String> = fresh
        .iter()
        .filter_map(|(name, content)| match committed.get(name) {
            None => Some(format!("missing {VIEWS_DIR}/{name}")),
            Some(existing) if existing != content => Some(format!("stale {VIEWS_DIR}/{name}")),
            Some(_) => None,
        })
        .collect();
    findings.extend(
        committed
            .keys()
            .filter(|name| !fresh.contains_key(*name) && name.as_str() != HOT_PATH)
            .map(|name| {
                format!("unexpected {VIEWS_DIR}/{name}: views are generated, not authored")
            }),
    );
    findings
}

#[cfg(test)]
mod tests {
    use super::{Language, Verdict, comment_findings, core_findings, language, stale_views};
    use crate::views::Files;
    use crate::workspace::{Member, Package, Workspace};
    use std::collections::BTreeSet;

    fn package(name: &str) -> Package {
        Package {
            name: name.to_string(),
            version: String::from("1.0.0"),
        }
    }

    fn with_core(dependencies: &[&str]) -> Workspace {
        Workspace {
            members: vec![Member {
                package: package("phonepad-core"),
                manifest: String::from("crates/phonepad-core/Cargo.toml"),
                dependencies: dependencies
                    .iter()
                    .map(|d| package(d))
                    .collect::<BTreeSet<Package>>(),
            }],
        }
    }

    fn files(entries: &[(&str, &str)]) -> Files {
        entries
            .iter()
            .map(|(n, c)| (n.to_string(), c.to_string()))
            .collect()
    }

    #[test]
    fn a_core_without_dependencies_has_no_findings() {
        assert_eq!(core_findings(&with_core(&[])), Ok(vec![]));
    }

    #[test]
    fn each_core_dependency_is_a_finding() {
        assert_eq!(
            core_findings(&with_core(&["libc", "tokio"])),
            Ok(vec![
                String::from(
                    "phonepad-core depends on libc 1.0.0 (crates/phonepad-core/Cargo.toml)"
                ),
                String::from(
                    "phonepad-core depends on tokio 1.0.0 (crates/phonepad-core/Cargo.toml)"
                ),
            ])
        );
    }

    #[test]
    fn a_workspace_without_core_is_an_error() {
        assert!(core_findings(&Workspace { members: vec![] }).is_err());
    }

    #[test]
    fn only_rust_files_and_the_phone_page_are_scanned() {
        assert_eq!(
            (
                language("a/b.rs").is_some(),
                language("pad.html").is_some(),
                language("x/pad.html").is_some(),
                language("server.py").is_some()
            ),
            (true, true, false, false)
        );
    }

    #[test]
    fn comment_findings_name_path_and_line() {
        assert_eq!(
            comment_findings("pad.html", Language::Html, "<p>\n<!-- a -->\n<!-- b -->\n"),
            vec![String::from("pad.html:2"), String::from("pad.html:3")]
        );
    }

    #[test]
    fn an_ambiguous_slash_is_a_finding() {
        assert_eq!(
            comment_findings(
                "pad.html",
                Language::Html,
                "<script>\nf() {}\n/x/;\n</script>\n"
            ),
            vec![String::from(
                "pad.html:3 `/` could start a regex or be a division; the lexer cannot classify it"
            )]
        );
    }

    #[test]
    fn rust_findings_use_the_rust_lexer() {
        assert_eq!(
            comment_findings("a.rs", Language::Rust, "<!-- no -->\n// yes\n"),
            vec![String::from("a.rs:2")]
        );
    }

    #[test]
    fn identical_views_are_fresh() {
        assert_eq!(
            stale_views(
                &files(&[("a.mmd", "x")]),
                &files(&[("a.mmd", "x"), ("hot-path.md", "y")])
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn hand_authored_files_beside_the_views_are_reported() {
        assert_eq!(
            stale_views(
                &files(&[("a.mmd", "x")]),
                &files(&[
                    ("a.mmd", "x"),
                    ("hand.mmd", "h"),
                    ("notes.md", "n"),
                    ("hot-path.md", "p")
                ])
            ),
            vec![
                String::from("unexpected docs/views/hand.mmd: views are generated, not authored"),
                String::from("unexpected docs/views/notes.md: views are generated, not authored"),
            ]
        );
    }

    #[test]
    fn changed_and_absent_views_are_reported() {
        assert_eq!(
            stale_views(
                &files(&[("a.mmd", "x"), ("b.mmd", "y"), ("c.mmd", "z")]),
                &files(&[("a.mmd", "old"), ("c.mmd", "z")])
            ),
            vec![
                String::from("stale docs/views/a.mmd"),
                String::from("missing docs/views/b.mmd")
            ]
        );
    }

    #[test]
    fn no_findings_pass() {
        assert_eq!(Verdict::from(Ok(vec![])), Verdict::Pass);
    }

    #[test]
    fn findings_fail_with_each_finding() {
        assert_eq!(
            Verdict::from(Ok(vec![String::from("a:1"), String::from("b:2")])),
            Verdict::Fail(vec![String::from("a:1"), String::from("b:2")])
        );
    }

    #[test]
    fn an_error_fails() {
        assert_eq!(
            Verdict::from(Err(String::from("no git"))),
            Verdict::Fail(vec![String::from("error: no git")])
        );
    }
}
