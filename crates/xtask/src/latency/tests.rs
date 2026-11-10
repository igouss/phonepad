use super::{DEFAULT_PORT, Phase, Sample, acked, calibration, link, phases, plan, runtime_name};

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

fn rates(words: &[&str]) -> Result<Vec<(String, Option<u32>)>, String> {
    phases(&plan(words)?).map(|phases: Vec<Phase>| {
        phases
            .into_iter()
            .map(|phase: Phase| (phase.profile.name, phase.hz))
            .collect()
    })
}

#[test]
fn live_phases_have_no_content_rate() {
    assert_eq!(rates(&["lte"]), Ok(vec![(String::from("lte"), None)]));
}

#[test]
fn nested_phases_all_paint_at_the_planned_rate() {
    assert_eq!(
        rates(&["nested", "hz=125", "viewer", "lan", "lte"]),
        Ok(vec![
            (String::from("lan"), Some(125)),
            (String::from("lte"), Some(125))
        ])
    );
}

#[test]
fn the_content_known_case_paints_lan_still_slow_and_fast() {
    assert_eq!(
        rates(&["nested", "calibrate"]),
        Ok(vec![
            (String::from("lan"), Some(0)),
            (String::from("lan"), Some(20)),
            (String::from("lan"), Some(125))
        ])
    );
}

#[test]
fn the_emulator_known_case_has_no_content_rate() {
    assert_eq!(
        rates(&["calibrate"]).map(|phases: Vec<(String, Option<u32>)>| phases
            .into_iter()
            .map(|(_, hz)| hz)
            .collect::<Vec<Option<u32>>>()),
        Ok(vec![None, None, None, None])
    );
}

fn unit(id: u32) -> Sample {
    Sample::Unit {
        id,
        bytes: 100,
        capture_send: 1.0,
        send_ack: 1.0,
    }
}

const DRAWN: Sample = Sample::Drawn {
    decode: 0.0,
    draw: 0.0,
    gap: None,
};

#[test]
fn no_samples_ack_nothing() {
    assert_eq!(acked(&[]), Vec::<u32>::new());
}

#[test]
fn one_unit_acks_its_id() {
    assert_eq!(acked(&[unit(7)]), vec![7]);
}

#[test]
fn many_units_ack_their_ids_in_order_and_draws_ack_nothing() {
    assert_eq!(acked(&[unit(3), DRAWN, unit(4), unit(9)]), vec![3, 4, 9]);
}
