use crate::decision::Decision;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wallpaper {
    Idle,
    Yielded,
    Held,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    Absent,
    Stale,
    Live,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observation {
    pub manual_pause: bool,
    pub marker: Marker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallpaperInput {
    Startup { marker: Marker },
    CaptureStarted(Observation),
    Observed(Observation),
    CaptureStopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallpaperEvent {
    Recovered,
    Took,
    Adopted,
    Yielded,
    Reasserted,
    Released,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallpaperCommand {
    Pause,
    Resume,
    TakeMarker,
    ClearMarker,
}

fn decision(
    event: WallpaperEvent,
    commands: Vec<WallpaperCommand>,
) -> Decision<WallpaperEvent, WallpaperCommand> {
    Decision {
        events: vec![event],
        commands,
    }
}

fn claim(observation: Observation) -> Option<Decision<WallpaperEvent, WallpaperCommand>> {
    match observation {
        Observation {
            manual_pause: false,
            marker: Marker::Absent | Marker::Stale,
        } => Some(decision(
            WallpaperEvent::Took,
            vec![WallpaperCommand::TakeMarker, WallpaperCommand::Pause],
        )),
        Observation {
            manual_pause: true,
            marker: Marker::Stale,
        } => Some(decision(
            WallpaperEvent::Adopted,
            vec![WallpaperCommand::TakeMarker],
        )),
        Observation {
            manual_pause: false | true,
            marker: Marker::Live,
        }
        | Observation {
            manual_pause: true,
            marker: Marker::Absent,
        } => None,
    }
}

pub fn decide(
    state: &Wallpaper,
    input: WallpaperInput,
) -> Decision<WallpaperEvent, WallpaperCommand> {
    match (*state, input) {
        (
            Wallpaper::Idle,
            WallpaperInput::Startup {
                marker: Marker::Stale,
            },
        ) => decision(
            WallpaperEvent::Recovered,
            vec![WallpaperCommand::Resume, WallpaperCommand::ClearMarker],
        ),
        (Wallpaper::Idle, WallpaperInput::CaptureStarted(observation)) => {
            claim(observation).unwrap_or_else(|| decision(WallpaperEvent::Yielded, Vec::new()))
        }
        (
            Wallpaper::Yielded,
            WallpaperInput::CaptureStarted(observation) | WallpaperInput::Observed(observation),
        ) => claim(observation).unwrap_or_else(Decision::none),
        (
            Wallpaper::Held,
            WallpaperInput::CaptureStarted(Observation {
                manual_pause: false,
                ..
            })
            | WallpaperInput::Observed(Observation {
                manual_pause: false,
                ..
            }),
        ) => decision(WallpaperEvent::Reasserted, vec![WallpaperCommand::Pause]),
        (Wallpaper::Held, WallpaperInput::CaptureStopped) => decision(
            WallpaperEvent::Released,
            vec![WallpaperCommand::Resume, WallpaperCommand::ClearMarker],
        ),
        (Wallpaper::Yielded, WallpaperInput::CaptureStopped) => {
            decision(WallpaperEvent::Stopped, Vec::new())
        }
        (
            Wallpaper::Idle,
            WallpaperInput::Startup {
                marker: Marker::Absent | Marker::Live,
            },
        )
        | (Wallpaper::Idle, WallpaperInput::Observed(_) | WallpaperInput::CaptureStopped)
        | (Wallpaper::Yielded | Wallpaper::Held, WallpaperInput::Startup { .. })
        | (
            Wallpaper::Held,
            WallpaperInput::CaptureStarted(Observation {
                manual_pause: true, ..
            })
            | WallpaperInput::Observed(Observation {
                manual_pause: true, ..
            }),
        ) => Decision::none(),
    }
}

pub fn evolve(state: Wallpaper, event: &WallpaperEvent) -> Wallpaper {
    match (state, event) {
        (
            Wallpaper::Idle | Wallpaper::Yielded | Wallpaper::Held,
            WallpaperEvent::Took | WallpaperEvent::Adopted | WallpaperEvent::Reasserted,
        ) => Wallpaper::Held,
        (Wallpaper::Idle | Wallpaper::Yielded | Wallpaper::Held, WallpaperEvent::Yielded) => {
            Wallpaper::Yielded
        }
        (
            Wallpaper::Idle | Wallpaper::Yielded | Wallpaper::Held,
            WallpaperEvent::Recovered | WallpaperEvent::Released | WallpaperEvent::Stopped,
        ) => Wallpaper::Idle,
    }
}

pub fn all_states() -> Vec<Wallpaper> {
    vec![Wallpaper::Idle, Wallpaper::Yielded, Wallpaper::Held]
}

fn all_markers() -> [Marker; 3] {
    [Marker::Absent, Marker::Stale, Marker::Live]
}

fn all_observations() -> Vec<Observation> {
    [false, true]
        .into_iter()
        .flat_map(|manual_pause: bool| {
            all_markers().map(|marker: Marker| Observation {
                manual_pause,
                marker,
            })
        })
        .collect()
}

pub fn all_inputs() -> Vec<WallpaperInput> {
    all_markers()
        .map(|marker: Marker| WallpaperInput::Startup { marker })
        .into_iter()
        .chain(
            all_observations()
                .into_iter()
                .map(WallpaperInput::CaptureStarted),
        )
        .chain(all_observations().into_iter().map(WallpaperInput::Observed))
        .chain([WallpaperInput::CaptureStopped])
        .collect()
}

#[cfg(test)]
mod tests;
