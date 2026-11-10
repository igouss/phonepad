use super::{DEFAULT_PORT, calibration, link, runtime_name};

#[test]
fn calibration_adds_100_and_200_ms_to_a_bare_link_then_caps_it_at_2_mbit() {
    assert_eq!(
        calibration().map(|phases: Vec<link::Profile>| phases
            .into_iter()
            .map(|p| (p.rtt.as_millis(), p.rate.map(|r| r.get())))
            .collect::<Vec<(u128, Option<u64>)>>()),
        Ok(vec![
            (0, None),
            (100, None),
            (200, None),
            (0, Some(2_000_000))
        ])
    );
}

#[test]
fn the_default_port_keeps_the_default_runtime_dir() {
    assert_eq!(runtime_name(DEFAULT_PORT), "pplat");
}

#[test]
fn another_port_gets_its_own_runtime_dir() {
    assert_eq!(runtime_name(8781), "pplat8781");
}
