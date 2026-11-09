use crate::lexer::{self, Scan};
use crate::profile::HOT_PATH;
use crate::views::{Files, VIEWS_DIR, read_dir_files, regenerate};
use crate::workspace::{self, Member, Workspace};
use std::collections::BTreeMap;
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
    let workspace: Workspace = workspace::load(root)?;
    let core: &Member = workspace
        .member(CORE)
        .ok_or_else(|| format!("{CORE} is not a workspace member"))?;
    let core_src: PathBuf = root
        .join(&core.manifest)
        .parent()
        .map(|dir| dir.join("src"))
        .ok_or_else(|| format!("{}: no parent directory", core.manifest))?;
    let mut findings: Vec<String> = stale_views(&fresh, &committed);
    findings.extend(unwired_machines(&machine_modules(root, &core_src)?, &fresh));
    Ok(findings)
}

fn declares_machine(source: &str) -> bool {
    source
        .lines()
        .any(|line| line.trim_start().starts_with("pub fn all_states("))
}

fn machine_modules(root: &Path, src: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut modules: BTreeMap<String, String> = BTreeMap::new();
    let mut pending: Vec<PathBuf> = vec![src.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries: std::fs::ReadDir =
            std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let path: PathBuf = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let source: String = std::fs::read_to_string(&path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                if declares_machine(&source) {
                    let relative: &Path = path.strip_prefix(src).map_err(|e| e.to_string())?;
                    let module: String = relative
                        .with_extension("")
                        .to_string_lossy()
                        .replace(std::path::MAIN_SEPARATOR, "::");
                    let shown: &Path = path.strip_prefix(root).unwrap_or(&path);
                    modules.insert(module, shown.display().to_string());
                }
            }
        }
    }
    Ok(modules)
}

fn unwired_machines(modules: &BTreeMap<String, String>, fresh: &Files) -> Vec<String> {
    modules
        .iter()
        .filter(|(module, _)| !fresh.contains_key(&format!("{}.mmd", module.replace("::", "-"))))
        .map(|(module, path)| {
            format!("unwired machine {module} ({path}) declares all_states but the generator emits no diagram for it")
        })
        .collect()
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
    use super::{
        Language, Verdict, comment_findings, core_findings, declares_machine, language,
        stale_views, unwired_machines,
    };
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

    fn modules(names: &[&str]) -> std::collections::BTreeMap<String, String> {
        names
            .iter()
            .map(|n| (n.to_string(), format!("src/{n}.rs")))
            .collect()
    }

    #[test]
    fn a_module_without_all_states_is_not_a_machine() {
        assert!(!declares_machine(
            "pub fn decide() {}\nfn all_states() {}\n"
        ));
    }

    #[test]
    fn a_module_with_public_all_states_is_a_machine() {
        assert!(declares_machine(
            "pub fn decide() {}\n    pub fn all_states() -> Vec<S> {\n"
        ));
    }

    #[test]
    fn no_machines_are_never_unwired() {
        assert_eq!(
            unwired_machines(&modules(&[]), &files(&[("a.mmd", "x")])),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_wired_machine_is_not_reported() {
        assert_eq!(
            unwired_machines(&modules(&["stream"]), &files(&[("stream.mmd", "x")])),
            Vec::<String>::new()
        );
    }

    #[test]
    fn every_machine_without_a_diagram_is_reported() {
        assert_eq!(
            unwired_machines(
                &modules(&["clock", "stream", "valve"]),
                &files(&[("stream.mmd", "x")])
            ),
            vec![
                String::from(
                    "unwired machine clock (src/clock.rs) declares all_states but the generator emits no diagram for it"
                ),
                String::from(
                    "unwired machine valve (src/valve.rs) declares all_states but the generator emits no diagram for it"
                ),
            ]
        );
    }

    #[test]
    fn every_core_machine_is_wired_into_the_generator() {
        let root: std::path::PathBuf = crate::views::own_root().unwrap();
        let found: Vec<String> =
            super::machine_modules(&root, &root.join("crates/phonepad-core/src"))
                .unwrap()
                .into_keys()
                .collect();
        let wired: Vec<String> = crate::views::machines().into_keys().collect();
        assert_eq!(found, wired);
    }
}
