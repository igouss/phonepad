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
            commands: vec![WallpaperCommand::WriteMarker, WallpaperCommand::Pause],
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
mod tests;
