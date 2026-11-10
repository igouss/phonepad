use super::{Plan, calibration, link, plan};

#[test]
fn no_words_is_no_plan() {
    assert!(plan(&[]).is_err());
}

#[test]
fn calibrate_is_the_known_case_plan() {
    assert_eq!(plan(&["calibrate"]), Ok(Plan::Calibrate));
}

#[test]
fn one_profile_is_measured() {
    assert_eq!(
        plan(&["lte"]),
        Ok(Plan::Profiles(vec![link::named("lte").unwrap()]))
    );
}

#[test]
fn many_profiles_are_measured_in_order() {
    assert_eq!(
        plan(&["bad-lte", "lan"]).map(|p: Plan| match p {
            Plan::Profiles(profiles) => profiles.into_iter().map(|p| p.name).collect(),
            Plan::Calibrate => Vec::new(),
        }),
        Ok(vec![String::from("bad-lte"), String::from("lan")])
    );
}

#[test]
fn an_unknown_profile_is_rejected() {
    assert!(plan(&["lan", "5g"]).is_err());
}

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
