use super::{executable, instance, listening, spawned};
use std::ffi::OsStr;
use std::path::PathBuf;

#[test]
fn the_listen_line_names_the_port() {
    assert!(listening(
        "phonepad listening on http://127.0.0.1:8769",
        8769
    ));
}

#[test]
fn another_port_is_not_listening() {
    assert!(!listening(
        "phonepad listening on http://127.0.0.1:8765",
        8769
    ));
}

fn live(signatures: &[&str]) -> Vec<String> {
    signatures.iter().map(|s: &&str| (*s).to_owned()).collect()
}

#[test]
fn no_live_instance_is_an_error() {
    assert!(instance(Some("a"), &[]).is_err());
}

#[test]
fn a_stale_inherited_signature_yields_the_one_live_instance() {
    assert_eq!(
        instance(Some("stale"), &live(&["live"])),
        Ok(String::from("live"))
    );
}

#[test]
fn no_inherited_signature_yields_the_one_live_instance() {
    assert_eq!(instance(None, &live(&["live"])), Ok(String::from("live")));
}

#[test]
fn a_live_inherited_signature_wins_among_many() {
    assert_eq!(
        instance(Some("b"), &live(&["a", "b", "c"])),
        Ok(String::from("b"))
    );
}

#[test]
fn many_live_instances_without_a_live_inherited_one_are_ambiguous() {
    assert!(instance(Some("stale"), &live(&["a", "b"])).is_err());
}

fn argv() -> [&'static OsStr; 3] {
    [
        OsStr::new("/b/phonepad"),
        OsStr::new("8769"),
        OsStr::new("test@example.invalid"),
    ]
}

#[test]
fn the_spawned_command_line_matches() {
    assert!(spawned(
        b"/b/phonepad\x008769\x00test@example.invalid\x00",
        &argv()
    ));
}

#[test]
fn an_empty_command_line_is_not_ours() {
    assert!(!spawned(b"", &argv()));
}

#[test]
fn another_port_is_not_ours() {
    assert!(!spawned(
        b"/b/phonepad\x008765\x00test@example.invalid\x00",
        &argv()
    ));
}

#[test]
fn an_extra_argument_is_not_ours() {
    assert!(!spawned(
        b"/b/phonepad\x008769\x00test@example.invalid\x00x\x00",
        &argv()
    ));
}

const ARTIFACT: &str = r#"{"reason":"compiler-artifact","target":{"name":"phonepad"},"executable":"/t/debug/phonepad"}"#;

#[test]
fn no_cargo_messages_name_no_executable() {
    assert_eq!(executable("", "phonepad"), None);
}

#[test]
fn the_artifact_names_the_executable() {
    assert_eq!(
        executable(ARTIFACT, "phonepad"),
        Some(PathBuf::from("/t/debug/phonepad"))
    );
}

#[test]
fn other_targets_and_reasons_are_skipped() {
    let messages: String = [
        r#"{"reason":"compiler-artifact","target":{"name":"phonepad-core"},"executable":"/t/x"}"#,
        r#"{"reason":"build-finished","target":{"name":"phonepad"},"executable":"/t/y"}"#,
        "not json",
        ARTIFACT,
        r#"{"reason":"compiler-artifact","target":{"name":"phonepad"},"executable":null}"#,
    ]
    .join("\n");
    assert_eq!(
        executable(&messages, "phonepad"),
        Some(PathBuf::from("/t/debug/phonepad"))
    );
}

#[test]
fn a_later_artifact_of_another_target_is_skipped() {
    let messages: String = [
        ARTIFACT,
        r#"{"reason":"compiler-artifact","target":{"name":"xtask"},"executable":"/t/debug/xtask"}"#,
    ]
    .join("\n");
    assert_eq!(
        executable(&messages, "phonepad"),
        Some(PathBuf::from("/t/debug/phonepad"))
    );
}
