use super::{DEFAULT_HZ, DEFAULT_MEASURED, DEFAULT_PORT, Desktop, Plan, Run, link, plan};
use crate::latency::nested::STOCK;
use std::path::PathBuf;
use std::time::Duration;

fn default(run: Run) -> Plan {
    Plan {
        run,
        port: DEFAULT_PORT,
        measured: DEFAULT_MEASURED,
        desktop: Desktop::Live,
    }
}

#[test]
fn no_words_is_no_plan() {
    assert!(plan(&[]).is_err());
}

#[test]
fn calibrate_is_the_known_case_plan() {
    assert_eq!(plan(&["calibrate"]), Ok(default(Run::Calibrate)));
}

#[test]
fn one_profile_is_measured() {
    assert_eq!(
        plan(&["lte"]),
        Ok(default(Run::Page(vec![link::named("lte").unwrap()])))
    );
}

#[test]
fn many_profiles_are_measured_in_order() {
    assert_eq!(
        plan(&["bad-lte", "lan"]).map(|p: Plan| match p.run {
            Run::Page(profiles) => profiles.into_iter().map(|p| p.name).collect(),
            Run::Viewer(_) | Run::Touch(_) | Run::Calibrate | Run::Content => Vec::new(),
        }),
        Ok(vec![String::from("bad-lte"), String::from("lan")])
    );
}

#[test]
fn an_unknown_profile_is_rejected() {
    assert!(plan(&["lan", "5g"]).is_err());
}

#[test]
fn viewer_measures_the_profiles_with_the_acking_viewer() {
    assert_eq!(
        plan(&["viewer", "lte"]),
        Ok(default(Run::Viewer(vec![link::named("lte").unwrap()])))
    );
}

#[test]
fn calibrate_with_viewer_is_rejected() {
    assert!(plan(&["viewer", "calibrate"]).is_err());
}

#[test]
fn port_and_seconds_override_the_defaults_wherever_they_stand() {
    assert_eq!(
        plan(&["lan", "port=8781", "seconds=60"]),
        Ok(Plan {
            run: Run::Page(vec![link::named("lan").unwrap()]),
            port: 8781,
            measured: Duration::from_secs(60),
            desktop: Desktop::Live,
        })
    );
}

#[test]
fn zero_seconds_is_rejected() {
    assert!(plan(&["lan", "seconds=0"]).is_err());
}

#[test]
fn one_second_is_measured() {
    assert_eq!(
        plan(&["lan", "seconds=1"]).map(|p: Plan| p.measured),
        Ok(Duration::from_secs(1))
    );
}

#[test]
fn a_port_that_is_not_a_number_is_rejected() {
    assert!(plan(&["lan", "port=http"]).is_err());
}

#[test]
fn options_alone_are_no_plan() {
    assert!(plan(&["viewer", "port=8781"]).is_err());
}

#[test]
fn touch_measures_the_profiles_by_touching_the_page() {
    assert_eq!(
        plan(&["touch", "bad-lte"]),
        Ok(default(Run::Touch(vec![link::named("bad-lte").unwrap()])))
    );
}

#[test]
fn touch_with_the_viewer_is_rejected() {
    assert!(plan(&["touch", "viewer", "lte"]).is_err());
}

#[test]
fn touch_calibrate_is_rejected() {
    assert!(plan(&["touch", "calibrate"]).is_err());
}

fn desktop(words: &[&str]) -> Result<Desktop, String> {
    plan(words).map(|p: Plan| p.desktop)
}

#[test]
fn nested_alone_runs_the_stock_hyprland_at_the_default_rate() {
    assert_eq!(
        desktop(&["nested", "lte"]),
        Ok(Desktop::Nested {
            hyprland: PathBuf::from(STOCK),
            hz: DEFAULT_HZ
        })
    );
}

#[test]
fn hyprland_names_another_build() {
    assert_eq!(
        desktop(&["nested", "hyprland=/b/Hyprland", "lte"]),
        Ok(Desktop::Nested {
            hyprland: PathBuf::from("/b/Hyprland"),
            hz: DEFAULT_HZ
        })
    );
}

#[test]
fn hz_sets_the_content_rate() {
    assert_eq!(
        desktop(&["nested", "hz=125", "lte"]),
        Ok(Desktop::Nested {
            hyprland: PathBuf::from(STOCK),
            hz: 125
        })
    );
}

#[test]
fn zero_hz_is_still_content() {
    assert_eq!(
        desktop(&["nested", "hz=0", "lan"]),
        Ok(Desktop::Nested {
            hyprland: PathBuf::from(STOCK),
            hz: 0
        })
    );
}

#[test]
fn hz_that_is_not_a_number_is_rejected() {
    assert!(plan(&["nested", "hz=fast", "lan"]).is_err());
}

#[test]
fn hz_without_nested_is_rejected() {
    assert!(plan(&["hz=60", "lan"]).is_err());
}

#[test]
fn hyprland_without_nested_is_rejected() {
    assert!(plan(&["hyprland=/b/Hyprland", "lan"]).is_err());
}

#[test]
fn nested_calibrate_is_the_content_known_case() {
    assert_eq!(
        plan(&["nested", "calibrate"]).map(|p: Plan| p.run),
        Ok(Run::Content)
    );
}

#[test]
fn nested_calibrate_with_a_rate_is_rejected() {
    assert!(plan(&["nested", "hz=60", "calibrate"]).is_err());
}

#[test]
fn nested_viewer_calibrate_is_rejected() {
    assert!(plan(&["nested", "viewer", "calibrate"]).is_err());
}

#[test]
fn touch_on_a_nested_hyprland_is_rejected() {
    assert!(plan(&["nested", "touch", "lte"]).is_err());
}
