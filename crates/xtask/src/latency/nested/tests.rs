use super::{Output, Placed, found, lock, output, placed, tail};
use std::path::{Path, PathBuf};

#[test]
fn an_empty_lock_names_no_instance() {
    assert_eq!(lock(""), None);
}

#[test]
fn a_lock_names_the_pid_and_its_wayland_socket() {
    assert_eq!(
        lock("2303812\nwayland-2\n"),
        Some((2303812, String::from("wayland-2")))
    );
}

#[test]
fn a_lock_without_a_socket_names_no_instance() {
    assert_eq!(lock("2303812\n"), None);
}

#[test]
fn a_lock_with_a_blank_socket_names_no_instance() {
    assert_eq!(lock("2303812\n \n"), None);
}

#[test]
fn a_lock_whose_pid_is_not_a_number_names_no_instance() {
    assert_eq!(lock("hypr\nwayland-2\n"), None);
}

#[test]
fn one_monitor_is_the_output() {
    assert_eq!(
        output(r#"[{"width":1254,"height":1414,"refreshRate":59.951}]"#),
        Ok(Output {
            width: 1254,
            height: 1414,
            refresh_hz: 59.951
        })
    );
}

#[test]
fn no_monitor_is_no_output() {
    assert!(output("[]").is_err());
}

#[test]
fn many_monitors_are_no_output() {
    assert!(
        output(
            r#"[{"width":1,"height":1,"refreshRate":60.0},{"width":1,"height":1,"refreshRate":60.0}]"#
        )
        .is_err()
    );
}

#[test]
fn a_monitor_without_a_refresh_rate_is_no_output() {
    assert!(output(r#"[{"width":1254,"height":1414}]"#).is_err());
}

#[test]
fn a_monitor_without_a_height_is_no_output() {
    assert!(output(r#"[{"width":1254,"refreshRate":60.0}]"#).is_err());
}

#[test]
fn monitors_that_are_not_a_list_are_no_output() {
    assert!(output(r#"{"width":1254}"#).is_err());
}

#[test]
fn no_clients_are_placed_nowhere() {
    assert_eq!(placed("[]"), Ok(Vec::new()));
}

#[test]
fn one_client_is_placed_at_its_position_and_size() {
    assert_eq!(
        placed(r#"[{"at":[0,0],"size":[1254,1414],"class":""}]"#),
        Ok(vec![Placed {
            at: (0, 0),
            size: (1254, 1414)
        }])
    );
}

#[test]
fn many_clients_are_placed_in_order() {
    assert_eq!(
        placed(r#"[{"at":[0,0],"size":[627,1414]},{"at":[627,0],"size":[627,1414]}]"#),
        Ok(vec![
            Placed {
                at: (0, 0),
                size: (627, 1414)
            },
            Placed {
                at: (627, 0),
                size: (627, 1414)
            }
        ])
    );
}

#[test]
fn a_client_without_a_size_is_an_error() {
    assert!(placed(r#"[{"at":[0,0]}]"#).is_err());
}

#[test]
fn clients_that_are_not_a_list_are_an_error() {
    assert!(placed(r#"{"at":[0,0]}"#).is_err());
}

struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn dir(name: &str) -> Dir {
    let path: PathBuf = std::env::temp_dir().join(format!(
        "phonepad-nested-test-{name}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&path).unwrap();
    Dir(path)
}

fn instance(hypr: &Path, signature: &str, lock: &str) {
    std::fs::create_dir(hypr.join(signature)).unwrap();
    std::fs::write(hypr.join(signature).join("hyprland.lock"), lock).unwrap();
}

#[test]
fn no_instance_is_found_in_an_empty_dir() {
    let hypr: Dir = dir("empty");
    assert_eq!(found(&hypr.0, 42), None);
}

#[test]
fn the_instance_whose_lock_names_the_pid_is_found() {
    let hypr: Dir = dir("one");
    instance(&hypr.0, "mine", "42\nwayland-3\n");
    assert_eq!(
        found(&hypr.0, 42),
        Some((String::from("mine"), String::from("wayland-3")))
    );
}

#[test]
fn another_pid_s_instance_is_not_found() {
    let hypr: Dir = dir("other");
    instance(&hypr.0, "theirs", "41\nwayland-1\n");
    assert_eq!(found(&hypr.0, 42), None);
}

#[test]
fn among_many_instances_only_the_pid_s_is_found() {
    let hypr: Dir = dir("many");
    instance(&hypr.0, "theirs", "41\nwayland-1\n");
    instance(&hypr.0, "mine", "42\nwayland-3\n");
    assert_eq!(
        found(&hypr.0, 42),
        Some((String::from("mine"), String::from("wayland-3")))
    );
}

#[test]
fn a_missing_log_has_an_empty_tail() {
    let logs: Dir = dir("nolog");
    assert_eq!(tail(&logs.0.join("hyprland.log")), "");
}

#[test]
fn a_long_log_keeps_its_last_ten_lines() {
    let logs: Dir = dir("log");
    let log: PathBuf = logs.0.join("hyprland.log");
    std::fs::write(&log, "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n").unwrap();
    assert_eq!(tail(&log), "3\n4\n5\n6\n7\n8\n9\n10\n11\n12");
}
