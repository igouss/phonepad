use crate::decision::Decision;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wallpaper {
    Untouched,
    PausedByUs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallpaperInput {
    Startup { marker_present: bool },
    CaptureStarted { manual_pause: bool },
    CaptureStopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallpaperEvent {
    Paused,
    Resumed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallpaperCommand {
    Pause,
    Resume,
    WriteMarker,
    RemoveMarker,
}

fn resume() -> Decision<WallpaperEvent, WallpaperCommand> {
    Decision {
        events: vec![WallpaperEvent::Resumed],
        commands: vec![WallpaperCommand::Resume, WallpaperCommand::RemoveMarker],
    }
}

pub fn decide(
    state: &Wallpaper,
    input: WallpaperInput,
) -> Decision<WallpaperEvent, WallpaperCommand> {
    match (*state, input) {
        (
            Wallpaper::Untouched | Wallpaper::PausedByUs,
            WallpaperInput::Startup {
                marker_present: true,
            },
        )
        | (Wallpaper::PausedByUs, WallpaperInput::CaptureStopped) => resume(),
        (
            Wallpaper::Untouched,
            WallpaperInput::CaptureStarted {
                manual_pause: false,
            },
        ) => Decision {
            events: vec![WallpaperEvent::Paused],
            commands: vec![WallpaperCommand::Pause, WallpaperCommand::WriteMarker],
        },
        (
            Wallpaper::Untouched | Wallpaper::PausedByUs,
            WallpaperInput::Startup {
                marker_present: false,
            },
        )
        | (
            Wallpaper::Untouched | Wallpaper::PausedByUs,
            WallpaperInput::CaptureStarted { manual_pause: true },
        )
        | (
            Wallpaper::PausedByUs,
            WallpaperInput::CaptureStarted {
                manual_pause: false,
            },
        )
        | (Wallpaper::Untouched, WallpaperInput::CaptureStopped) => Decision::none(),
    }
}

pub fn evolve(state: Wallpaper, event: &WallpaperEvent) -> Wallpaper {
    match (state, event) {
        (Wallpaper::Untouched | Wallpaper::PausedByUs, WallpaperEvent::Paused) => {
            Wallpaper::PausedByUs
        }
        (Wallpaper::Untouched | Wallpaper::PausedByUs, WallpaperEvent::Resumed) => {
            Wallpaper::Untouched
        }
    }
}

pub fn all_states() -> Vec<Wallpaper> {
    vec![Wallpaper::Untouched, Wallpaper::PausedByUs]
}

pub fn all_inputs() -> Vec<WallpaperInput> {
    vec![
        WallpaperInput::Startup {
            marker_present: false,
        },
        WallpaperInput::Startup {
            marker_present: true,
        },
        WallpaperInput::CaptureStarted {
            manual_pause: false,
        },
        WallpaperInput::CaptureStarted { manual_pause: true },
        WallpaperInput::CaptureStopped,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn variant_index(state: &Wallpaper) -> usize {
        match state {
            Wallpaper::Untouched => 0,
            Wallpaper::PausedByUs => 1,
        }
    }

    fn input_index(input: &WallpaperInput) -> usize {
        match input {
            WallpaperInput::Startup { .. } => 0,
            WallpaperInput::CaptureStarted { .. } => 1,
            WallpaperInput::CaptureStopped => 2,
        }
    }

    fn reached(inputs: &[WallpaperInput]) -> Wallpaper {
        inputs.iter().fold(
            Wallpaper::Untouched,
            |state: Wallpaper, input: &WallpaperInput| {
                decide(&state, *input).events.iter().fold(state, evolve)
            },
        )
    }

    fn paused() -> Decision<WallpaperEvent, WallpaperCommand> {
        Decision {
            events: vec![WallpaperEvent::Paused],
            commands: vec![WallpaperCommand::Pause, WallpaperCommand::WriteMarker],
        }
    }

    fn resumed() -> Decision<WallpaperEvent, WallpaperCommand> {
        Decision {
            events: vec![WallpaperEvent::Resumed],
            commands: vec![WallpaperCommand::Resume, WallpaperCommand::RemoveMarker],
        }
    }

    #[test]
    fn all_states_cover_every_variant() {
        assert_eq!(
            all_states()
                .iter()
                .map(variant_index)
                .collect::<BTreeSet<usize>>(),
            BTreeSet::from([0, 1])
        );
    }

    #[test]
    fn all_inputs_cover_every_variant() {
        assert_eq!(
            all_inputs()
                .iter()
                .map(input_index)
                .collect::<BTreeSet<usize>>(),
            BTreeSet::from([0, 1, 2])
        );
    }

    #[test]
    fn all_states_lists_every_state() {
        assert_eq!(
            all_states(),
            vec![Wallpaper::Untouched, Wallpaper::PausedByUs]
        );
    }

    #[test]
    fn all_inputs_lists_every_input_with_both_flags() {
        assert_eq!(
            all_inputs(),
            vec![
                WallpaperInput::Startup {
                    marker_present: false
                },
                WallpaperInput::Startup {
                    marker_present: true
                },
                WallpaperInput::CaptureStarted {
                    manual_pause: false
                },
                WallpaperInput::CaptureStarted { manual_pause: true },
                WallpaperInput::CaptureStopped,
            ]
        );
    }

    #[test]
    fn every_listed_state_is_reachable_from_untouched() {
        assert_eq!(
            all_states(),
            vec![
                reached(&[]),
                reached(&[WallpaperInput::CaptureStarted {
                    manual_pause: false
                }])
            ]
        );
    }

    #[test]
    fn startup_with_marker_resumes_untouched() {
        assert_eq!(
            decide(
                &Wallpaper::Untouched,
                WallpaperInput::Startup {
                    marker_present: true
                }
            ),
            resumed()
        );
    }

    #[test]
    fn startup_with_marker_resumes_paused() {
        assert_eq!(
            decide(
                &Wallpaper::PausedByUs,
                WallpaperInput::Startup {
                    marker_present: true
                }
            ),
            resumed()
        );
    }

    #[test]
    fn startup_without_marker_does_nothing_when_untouched() {
        assert_eq!(
            decide(
                &Wallpaper::Untouched,
                WallpaperInput::Startup {
                    marker_present: false
                }
            ),
            Decision::none()
        );
    }

    #[test]
    fn startup_without_marker_does_nothing_when_paused() {
        assert_eq!(
            decide(
                &Wallpaper::PausedByUs,
                WallpaperInput::Startup {
                    marker_present: false
                }
            ),
            Decision::none()
        );
    }

    #[test]
    fn capture_started_pauses_untouched_wallpaper() {
        assert_eq!(
            decide(
                &Wallpaper::Untouched,
                WallpaperInput::CaptureStarted {
                    manual_pause: false
                }
            ),
            paused()
        );
    }

    #[test]
    fn capture_started_when_already_paused_does_nothing() {
        assert_eq!(
            decide(
                &Wallpaper::PausedByUs,
                WallpaperInput::CaptureStarted {
                    manual_pause: false
                }
            ),
            Decision::none()
        );
    }

    #[test]
    fn capture_started_never_takes_over_manual_pause() {
        assert_eq!(
            decide(
                &Wallpaper::Untouched,
                WallpaperInput::CaptureStarted { manual_pause: true }
            ),
            Decision::none()
        );
    }

    #[test]
    fn capture_started_with_manual_pause_while_paused_does_nothing() {
        assert_eq!(
            decide(
                &Wallpaper::PausedByUs,
                WallpaperInput::CaptureStarted { manual_pause: true }
            ),
            Decision::none()
        );
    }

    #[test]
    fn capture_stopped_resumes_our_pause() {
        assert_eq!(
            decide(&Wallpaper::PausedByUs, WallpaperInput::CaptureStopped),
            resumed()
        );
    }

    #[test]
    fn capture_stopped_when_untouched_does_nothing() {
        assert_eq!(
            decide(&Wallpaper::Untouched, WallpaperInput::CaptureStopped),
            Decision::none()
        );
    }

    #[test]
    fn paused_marks_paused_by_us() {
        assert_eq!(
            evolve(Wallpaper::Untouched, &WallpaperEvent::Paused),
            Wallpaper::PausedByUs
        );
    }

    #[test]
    fn paused_again_stays_paused_by_us() {
        assert_eq!(
            evolve(Wallpaper::PausedByUs, &WallpaperEvent::Paused),
            Wallpaper::PausedByUs
        );
    }

    #[test]
    fn resumed_marks_untouched() {
        assert_eq!(
            evolve(Wallpaper::PausedByUs, &WallpaperEvent::Resumed),
            Wallpaper::Untouched
        );
    }

    #[test]
    fn resumed_again_stays_untouched() {
        assert_eq!(
            evolve(Wallpaper::Untouched, &WallpaperEvent::Resumed),
            Wallpaper::Untouched
        );
    }
}
