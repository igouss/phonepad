use super::{Output, Placed, lock, output, placed};

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
