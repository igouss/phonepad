use super::*;
use std::collections::BTreeSet;

fn variant_index(state: &Wallpaper) -> usize {
    match state {
        Wallpaper::Idle => 0,
        Wallpaper::Yielded => 1,
        Wallpaper::Held => 2,
    }
}

fn input_index(input: &WallpaperInput) -> usize {
    match input {
        WallpaperInput::Startup { .. } => 0,
        WallpaperInput::CaptureStarted(_) => 1,
        WallpaperInput::Observed(_) => 2,
        WallpaperInput::CaptureStopped => 3,
    }
}

fn reached(inputs: &[WallpaperInput]) -> Wallpaper {
    inputs.iter().fold(
        Wallpaper::Idle,
        |state: Wallpaper, input: &WallpaperInput| {
            decide(&state, *input).events.iter().fold(state, evolve)
        },
    )
}

fn seen(manual_pause: bool, marker: Marker) -> Observation {
    Observation {
        manual_pause,
        marker,
    }
}

fn took() -> Decision<WallpaperEvent, WallpaperCommand> {
    Decision {
        events: vec![WallpaperEvent::Took],
        commands: vec![WallpaperCommand::TakeMarker, WallpaperCommand::Pause],
    }
}

fn adopted() -> Decision<WallpaperEvent, WallpaperCommand> {
    Decision {
        events: vec![WallpaperEvent::Adopted],
        commands: vec![WallpaperCommand::TakeMarker],
    }
}

fn yielded() -> Decision<WallpaperEvent, WallpaperCommand> {
    Decision {
        events: vec![WallpaperEvent::Yielded],
        commands: Vec::new(),
    }
}

fn reasserted() -> Decision<WallpaperEvent, WallpaperCommand> {
    Decision {
        events: vec![WallpaperEvent::Reasserted],
        commands: vec![WallpaperCommand::Pause],
    }
}

#[test]
fn all_states_cover_every_variant() {
    assert_eq!(
        all_states()
            .iter()
            .map(variant_index)
            .collect::<BTreeSet<usize>>(),
        BTreeSet::from([0, 1, 2])
    );
}

#[test]
fn all_inputs_cover_every_variant() {
    assert_eq!(
        all_inputs()
            .iter()
            .map(input_index)
            .collect::<BTreeSet<usize>>(),
        BTreeSet::from([0, 1, 2, 3])
    );
}

#[test]
fn all_inputs_lists_three_markers_and_six_observations_twice() {
    assert_eq!(
        all_inputs(),
        vec![
            WallpaperInput::Startup {
                marker: Marker::Absent
            },
            WallpaperInput::Startup {
                marker: Marker::Stale
            },
            WallpaperInput::Startup {
                marker: Marker::Live
            },
            WallpaperInput::CaptureStarted(seen(false, Marker::Absent)),
            WallpaperInput::CaptureStarted(seen(false, Marker::Stale)),
            WallpaperInput::CaptureStarted(seen(false, Marker::Live)),
            WallpaperInput::CaptureStarted(seen(true, Marker::Absent)),
            WallpaperInput::CaptureStarted(seen(true, Marker::Stale)),
            WallpaperInput::CaptureStarted(seen(true, Marker::Live)),
            WallpaperInput::Observed(seen(false, Marker::Absent)),
            WallpaperInput::Observed(seen(false, Marker::Stale)),
            WallpaperInput::Observed(seen(false, Marker::Live)),
            WallpaperInput::Observed(seen(true, Marker::Absent)),
            WallpaperInput::Observed(seen(true, Marker::Stale)),
            WallpaperInput::Observed(seen(true, Marker::Live)),
            WallpaperInput::CaptureStopped,
        ]
    );
}

#[test]
fn every_listed_state_is_reachable_from_idle() {
    assert_eq!(
        all_states(),
        vec![
            reached(&[]),
            reached(&[WallpaperInput::CaptureStarted(seen(true, Marker::Absent))]),
            reached(&[WallpaperInput::CaptureStarted(seen(false, Marker::Absent))]),
        ]
    );
}

#[test]
fn startup_with_a_stale_marker_resumes_the_crashed_owners_pause() {
    assert_eq!(
        decide(
            &Wallpaper::Idle,
            WallpaperInput::Startup {
                marker: Marker::Stale
            }
        ),
        Decision {
            events: vec![WallpaperEvent::Recovered],
            commands: vec![WallpaperCommand::Resume, WallpaperCommand::ClearMarker],
        }
    );
}

#[test]
fn startup_without_a_marker_does_nothing() {
    assert_eq!(
        decide(
            &Wallpaper::Idle,
            WallpaperInput::Startup {
                marker: Marker::Absent
            }
        ),
        Decision::none()
    );
}

#[test]
fn startup_never_resumes_a_live_instances_pause() {
    assert_eq!(
        decide(
            &Wallpaper::Idle,
            WallpaperInput::Startup {
                marker: Marker::Live
            }
        ),
        Decision::none()
    );
}

#[test]
fn startup_while_capturing_does_nothing() {
    assert_eq!(
        [
            decide(
                &Wallpaper::Yielded,
                WallpaperInput::Startup {
                    marker: Marker::Stale
                }
            ),
            decide(
                &Wallpaper::Held,
                WallpaperInput::Startup {
                    marker: Marker::Stale
                }
            )
        ],
        [Decision::none(), Decision::none()]
    );
}

#[test]
fn capture_start_pauses_a_playing_wallpaper_and_takes_the_marker() {
    assert_eq!(
        decide(
            &Wallpaper::Idle,
            WallpaperInput::CaptureStarted(seen(false, Marker::Absent))
        ),
        took()
    );
}

#[test]
fn capture_start_over_a_stale_marker_takes_it_and_pauses() {
    assert_eq!(
        decide(
            &Wallpaper::Idle,
            WallpaperInput::CaptureStarted(seen(false, Marker::Stale))
        ),
        took()
    );
}

#[test]
fn capture_start_adopts_a_crashed_owners_pause() {
    assert_eq!(
        decide(
            &Wallpaper::Idle,
            WallpaperInput::CaptureStarted(seen(true, Marker::Stale))
        ),
        adopted()
    );
}

#[test]
fn capture_start_yields_to_a_pause_the_user_set() {
    assert_eq!(
        decide(
            &Wallpaper::Idle,
            WallpaperInput::CaptureStarted(seen(true, Marker::Absent))
        ),
        yielded()
    );
}

#[test]
fn capture_start_yields_to_a_live_instance_that_owns_the_pause() {
    assert_eq!(
        [
            decide(
                &Wallpaper::Idle,
                WallpaperInput::CaptureStarted(seen(true, Marker::Live))
            ),
            decide(
                &Wallpaper::Idle,
                WallpaperInput::CaptureStarted(seen(false, Marker::Live))
            )
        ],
        [yielded(), yielded()]
    );
}

#[test]
fn yielded_takes_over_when_the_other_pause_lapses() {
    assert_eq!(
        decide(
            &Wallpaper::Yielded,
            WallpaperInput::Observed(seen(false, Marker::Absent))
        ),
        took()
    );
}

#[test]
fn yielded_adopts_a_pause_whose_owner_crashed() {
    assert_eq!(
        decide(
            &Wallpaper::Yielded,
            WallpaperInput::Observed(seen(true, Marker::Stale))
        ),
        adopted()
    );
}

#[test]
fn yielded_leaves_a_users_pause_and_a_live_owner_alone() {
    assert_eq!(
        [
            decide(
                &Wallpaper::Yielded,
                WallpaperInput::Observed(seen(true, Marker::Absent))
            ),
            decide(
                &Wallpaper::Yielded,
                WallpaperInput::Observed(seen(false, Marker::Live))
            ),
            decide(
                &Wallpaper::Yielded,
                WallpaperInput::CaptureStarted(seen(true, Marker::Live))
            )
        ],
        [Decision::none(), Decision::none(), Decision::none()]
    );
}

#[test]
fn held_pause_that_lapsed_is_reasserted() {
    assert_eq!(
        [
            decide(
                &Wallpaper::Held,
                WallpaperInput::Observed(seen(false, Marker::Live))
            ),
            decide(
                &Wallpaper::Held,
                WallpaperInput::CaptureStarted(seen(false, Marker::Live))
            )
        ],
        [reasserted(), reasserted()]
    );
}

#[test]
fn held_pause_that_holds_needs_nothing() {
    assert_eq!(
        [
            decide(
                &Wallpaper::Held,
                WallpaperInput::Observed(seen(true, Marker::Live))
            ),
            decide(
                &Wallpaper::Held,
                WallpaperInput::CaptureStarted(seen(true, Marker::Live))
            )
        ],
        [Decision::none(), Decision::none()]
    );
}

#[test]
fn capture_stop_resumes_and_clears_only_our_pause() {
    assert_eq!(
        decide(&Wallpaper::Held, WallpaperInput::CaptureStopped),
        Decision {
            events: vec![WallpaperEvent::Released],
            commands: vec![WallpaperCommand::Resume, WallpaperCommand::ClearMarker],
        }
    );
}

#[test]
fn capture_stop_after_yielding_leaves_the_wallpaper_alone() {
    assert_eq!(
        decide(&Wallpaper::Yielded, WallpaperInput::CaptureStopped),
        Decision {
            events: vec![WallpaperEvent::Stopped],
            commands: Vec::new(),
        }
    );
}

#[test]
fn idle_ignores_observations_and_stops() {
    assert_eq!(
        [
            decide(
                &Wallpaper::Idle,
                WallpaperInput::Observed(seen(false, Marker::Absent))
            ),
            decide(&Wallpaper::Idle, WallpaperInput::CaptureStopped)
        ],
        [Decision::none(), Decision::none()]
    );
}

#[test]
fn claiming_events_lead_to_held() {
    assert_eq!(
        [
            evolve(Wallpaper::Idle, &WallpaperEvent::Took),
            evolve(Wallpaper::Yielded, &WallpaperEvent::Adopted),
            evolve(Wallpaper::Held, &WallpaperEvent::Reasserted)
        ],
        [Wallpaper::Held, Wallpaper::Held, Wallpaper::Held]
    );
}

#[test]
fn yielded_event_leads_to_yielded() {
    assert_eq!(
        evolve(Wallpaper::Idle, &WallpaperEvent::Yielded),
        Wallpaper::Yielded
    );
}

#[test]
fn ending_events_lead_to_idle() {
    assert_eq!(
        [
            evolve(Wallpaper::Idle, &WallpaperEvent::Recovered),
            evolve(Wallpaper::Held, &WallpaperEvent::Released),
            evolve(Wallpaper::Yielded, &WallpaperEvent::Stopped)
        ],
        [Wallpaper::Idle, Wallpaper::Idle, Wallpaper::Idle]
    );
}
