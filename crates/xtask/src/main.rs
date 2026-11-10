mod e2e;
mod gates;
mod h264_check;
mod lexer;
mod machine;
mod mermaid;
mod profile;
mod report;
mod testbed;
mod views;
mod workspace;

use gates::Verdict;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use views::VIEWS_DIR;

const USAGE: &str = "usage: cargo xtask views [--root <dir>] [--out <dir>]\n       cargo xtask check [--root <dir>]\n       cargo xtask h264-check   (needs the Intel iGPU at /dev/dri/renderD128 and the ffmpeg CLI)\n       cargo xtask e2e [--bin <phonepad>]   (needs a live Hyprland session, the iGPU, /dev/uinput and the ffmpeg CLI; serves on port 8769)\n       target/debug/xtask profile <pid> <seconds> [--root <dir>]\n         (run the binary directly: a cargo runner with its own PID namespace hides host processes)";

#[derive(Debug, PartialEq, Eq)]
enum Task {
    Views {
        root: Option<PathBuf>,
        out: Option<PathBuf>,
    },
    Check {
        root: Option<PathBuf>,
    },
    Profile {
        root: Option<PathBuf>,
        pid: u32,
        seconds: u32,
    },
    H264Check,
    E2e {
        bin: Option<PathBuf>,
    },
}

struct Args {
    positional: Vec<String>,
    root: Option<PathBuf>,
    out: Option<PathBuf>,
    bin: Option<PathBuf>,
}

fn split(args: &[String]) -> Result<Args, String> {
    let mut parsed: Args = Args {
        positional: Vec::new(),
        root: None,
        out: None,
        bin: None,
    };
    let mut rest: std::slice::Iter<String> = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--root" => {
                parsed.root = Some(PathBuf::from(
                    rest.next().ok_or("--root needs a directory")?,
                ))
            }
            "--out" => {
                parsed.out = Some(PathBuf::from(rest.next().ok_or("--out needs a directory")?))
            }
            "--bin" => {
                parsed.bin = Some(PathBuf::from(
                    rest.next().ok_or("--bin needs a phonepad binary")?,
                ))
            }
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            word => parsed.positional.push(word.to_string()),
        }
    }
    Ok(parsed)
}

fn number(text: &str, what: &str) -> Result<u32, String> {
    text.parse()
        .map_err(|_| format!("{what} must be a non-negative integer, got `{text}`"))
}

fn parse(args: &[String]) -> Result<Task, String> {
    let Args {
        positional,
        root,
        out,
        bin,
    } = split(args)?;
    let words: Vec<&str> = positional.iter().map(String::as_str).collect();
    match (words.as_slice(), out, bin) {
        (["views"], out, None) => Ok(Task::Views { root, out }),
        (["check"], None, None) => Ok(Task::Check { root }),
        (["h264-check"], None, None) if root.is_none() => Ok(Task::H264Check),
        (["e2e"], None, bin) if root.is_none() => Ok(Task::E2e { bin }),
        (["profile", pid, seconds], None, None) => Ok(Task::Profile {
            root,
            pid: number(pid, "pid")?,
            seconds: number(seconds, "seconds")?,
        }),
        _ => Err(String::from(USAGE)),
    }
}

fn root_or_own(root: Option<PathBuf>) -> Result<PathBuf, String> {
    match root {
        Some(root) => views::canonical(&root),
        None => views::own_root(),
    }
}

fn check(root: &Path) -> bool {
    let verdicts: Vec<(&str, Verdict)> = gates::all(root);
    for (gate, verdict) in &verdicts {
        match verdict {
            Verdict::Pass => println!("PASS {gate}"),
            Verdict::Fail(findings) => {
                println!("FAIL {gate}");
                findings.iter().for_each(|f| println!("  {f}"));
            }
        }
    }
    verdicts.iter().all(|(_, v)| *v == Verdict::Pass)
}

fn run(task: Task) -> Result<bool, String> {
    match task {
        Task::Views { root, out } => {
            let root: PathBuf = root_or_own(root)?;
            let out: PathBuf = out.unwrap_or_else(|| root.join(VIEWS_DIR));
            views::regenerate(&root, &out)?;
            println!("views written to {}", out.display());
            Ok(true)
        }
        Task::Check { root } => Ok(check(&root_or_own(root)?)),
        Task::Profile { root, pid, seconds } => {
            let written: PathBuf = profile::profile(&root_or_own(root)?, pid, seconds)?;
            println!("hot path written to {}", written.display());
            Ok(true)
        }
        Task::H264Check => Ok(h264_check::run()),
        Task::E2e { bin } => Ok(e2e::run(bin)),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args).and_then(run) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Task, parse};
    use std::path::PathBuf;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn no_arguments_is_a_usage_error() {
        assert!(parse(&[]).is_err());
    }

    #[test]
    fn check_takes_a_root() {
        assert_eq!(
            parse(&args("check --root /w")),
            Ok(Task::Check {
                root: Some(PathBuf::from("/w"))
            })
        );
    }

    #[test]
    fn views_takes_a_root_and_an_output() {
        assert_eq!(
            parse(&args("views --out /o --root /w")),
            Ok(Task::Views {
                root: Some(PathBuf::from("/w")),
                out: Some(PathBuf::from("/o"))
            })
        );
    }

    #[test]
    fn profile_takes_pid_and_seconds() {
        assert_eq!(
            parse(&args("profile 42 5")),
            Ok(Task::Profile {
                root: None,
                pid: 42,
                seconds: 5
            })
        );
    }

    #[test]
    fn profile_rejects_a_non_numeric_pid() {
        assert!(parse(&args("profile me 5")).is_err());
    }

    #[test]
    fn h264_check_takes_no_arguments() {
        assert_eq!(parse(&args("h264-check")), Ok(Task::H264Check));
    }

    #[test]
    fn h264_check_rejects_a_root() {
        assert!(parse(&args("h264-check --root /w")).is_err());
    }

    #[test]
    fn e2e_builds_its_own_binary_by_default() {
        assert_eq!(parse(&args("e2e")), Ok(Task::E2e { bin: None }));
    }

    #[test]
    fn e2e_takes_a_binary() {
        assert_eq!(
            parse(&args("e2e --bin /b/phonepad")),
            Ok(Task::E2e {
                bin: Some(PathBuf::from("/b/phonepad"))
            })
        );
    }

    #[test]
    fn e2e_rejects_a_root() {
        assert!(parse(&args("e2e --root /w")).is_err());
    }

    #[test]
    fn bin_without_a_path_is_rejected() {
        assert_eq!(
            parse(&args("e2e --bin")),
            Err(String::from("--bin needs a phonepad binary"))
        );
    }

    #[test]
    fn only_e2e_takes_a_binary() {
        assert!(parse(&args("check --bin /b/phonepad")).is_err());
    }

    #[test]
    fn unknown_options_are_rejected() {
        assert_eq!(
            parse(&args("check --fast")),
            Err(String::from("unknown option --fast"))
        );
    }
}
