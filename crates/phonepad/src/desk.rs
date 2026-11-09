use std::time::Duration;

use phonepad_core::decision::Decision;
use phonepad_core::desk::{self, Desk, DeskCommand, DeskEvent, DeskInput, PointerSize};
use phonepad_core::geometry::Point;
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};
use tokio::task::{self, JoinHandle};
use tokio::time::{self, Interval, MissedTickBehavior};

use crate::capture::{Capture, Feed};
use crate::clock::Clock;
use crate::hypr;
use crate::owe::WallpaperShell;

const LOCAL_MOUSE_POLL: Duration = Duration::from_millis(100);
const QUEUE: usize = 64;
const CAPTURE_STOP_BOUND: Duration = Duration::from_secs(1);
const CAPTURE_RESTART_BACKOFF: Duration = Duration::from_secs(1);

pub enum DeskMessage {
    Input(DeskInput),
    CaptureExited(u64),
    RestartCapture,
    Shutdown(oneshot::Sender<()>),
}

struct RunningCapture {
    generation: u64,
    capture: Capture,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PointerStyle {
    pub theme: String,
    pub normal: u32,
    pub big: u32,
}

pub struct DeskShell {
    desk: Desk,
    capture: Option<RunningCapture>,
    capture_generation: u64,
    local_mouse_watch: Option<JoinHandle<()>>,
    pointer: PointerStyle,
    wallpaper: WallpaperShell,
    clock: Clock,
    feed: Feed,
    inbox: mpsc::Sender<DeskMessage>,
}

pub fn spawn(pointer: PointerStyle, clock: Clock, feed: Feed) -> mpsc::Sender<DeskMessage> {
    let (inbox, receiver): (mpsc::Sender<DeskMessage>, mpsc::Receiver<DeskMessage>) =
        mpsc::channel(QUEUE);
    let shell: DeskShell = DeskShell {
        desk: Desk::initial(),
        capture: None,
        capture_generation: 0,
        local_mouse_watch: None,
        pointer,
        wallpaper: WallpaperShell::from_env(),
        clock,
        feed,
        inbox: inbox.clone(),
    };
    tokio::spawn(shell.run(receiver));
    inbox
}

impl DeskShell {
    async fn run(mut self, mut receiver: mpsc::Receiver<DeskMessage>) {
        self.set_pointer(PointerSize::Normal).await;
        self.wallpaper.startup().await;
        while let Some(message) = receiver.recv().await {
            match message {
                DeskMessage::Input(input) => self.step(input).await,
                DeskMessage::CaptureExited(generation) => self.capture_exited(generation).await,
                DeskMessage::RestartCapture => self.restart_capture(),
                DeskMessage::Shutdown(done) => {
                    self.shutdown().await;
                    let _ = done.send(());
                    return;
                }
            }
        }
    }

    async fn step(&mut self, input: DeskInput) {
        let decision: Decision<DeskEvent, DeskCommand> = desk::decide(&self.desk, input);
        self.desk = decision.events.iter().fold(self.desk, desk::evolve);
        for command in decision.commands {
            self.execute(command).await;
        }
    }

    async fn execute(&mut self, command: DeskCommand) {
        match command {
            DeskCommand::StartCapture => {
                self.start_capture();
                self.wallpaper.capture_started().await;
            }
            DeskCommand::StopCapture => self.stop_capture().await,
            DeskCommand::SetPointer(size) => self.set_pointer(size).await,
            DeskCommand::StartLocalMouseWatch => self.start_local_mouse_watch(),
            DeskCommand::StopLocalMouseWatch => self.stop_local_mouse_watch(),
        }
    }

    fn start_capture(&mut self) {
        if self.capture.is_some() {
            return;
        }
        self.capture_generation += 1;
        let generation: u64 = self.capture_generation;
        let inbox: mpsc::Sender<DeskMessage> = self.inbox.clone();
        let capture: Capture = Capture::start(
            self.feed.clone(),
            Box::new(move || {
                let _ = inbox.blocking_send(DeskMessage::CaptureExited(generation));
            }),
        );
        self.capture = Some(RunningCapture {
            generation,
            capture,
        });
    }

    async fn stop_capture(&mut self) {
        if let Some(running) = self.capture.take() {
            join_bounded(running.capture).await;
        }
        self.wallpaper.capture_stopped().await;
    }

    async fn capture_exited(&mut self, generation: u64) {
        let current: bool = self
            .capture
            .as_ref()
            .is_some_and(|running: &RunningCapture| running.generation == generation);
        if !current {
            return;
        }
        if let Some(running) = self.capture.take() {
            join_bounded(running.capture).await;
        }
        eprintln!("capture: stopped on its own; restarting in {CAPTURE_RESTART_BACKOFF:?}");
        let inbox: mpsc::Sender<DeskMessage> = self.inbox.clone();
        tokio::spawn(async move {
            time::sleep(CAPTURE_RESTART_BACKOFF).await;
            let _ = inbox.send(DeskMessage::RestartCapture).await;
        });
    }

    fn restart_capture(&mut self) {
        if self.desk.viewers > 0 {
            self.start_capture();
        }
    }

    async fn set_pointer(&self, size: PointerSize) {
        let pixels: String = match size {
            PointerSize::Normal => self.pointer.normal,
            PointerSize::Big => self.pointer.big,
        }
        .to_string();
        run(
            "gsettings",
            &["set", "org.gnome.desktop.interface", "cursor-size", &pixels],
        )
        .await;
        run("hyprctl", &["setcursor", &self.pointer.theme, &pixels]).await;
    }

    fn start_local_mouse_watch(&mut self) {
        self.stop_local_mouse_watch();
        self.local_mouse_watch = Some(tokio::spawn(watch_local_mouse(
            self.inbox.clone(),
            self.clock,
        )));
    }

    fn stop_local_mouse_watch(&mut self) {
        if let Some(watch) = self.local_mouse_watch.take() {
            watch.abort();
        }
    }

    async fn shutdown(&mut self) {
        self.stop_local_mouse_watch();
        self.stop_capture().await;
        self.set_pointer(PointerSize::Normal).await;
    }
}

async fn join_bounded(capture: Capture) {
    let threads: [std::thread::JoinHandle<()>; 2] = capture.stop();
    match time::timeout(
        CAPTURE_STOP_BOUND,
        task::spawn_blocking(move || threads.map(std::thread::JoinHandle::join)),
    )
    .await
    {
        Ok(Ok([Ok(()), Ok(())])) => {}
        Ok(Ok(_)) => eprintln!("capture: thread panicked"),
        Ok(Err(err)) => eprintln!("capture: join task failed: {err}"),
        Err(_) => {
            eprintln!("capture: thread still running {CAPTURE_STOP_BOUND:?} after stop; detached")
        }
    }
}

async fn watch_local_mouse(inbox: mpsc::Sender<DeskMessage>, clock: Clock) {
    let mut ticks: Interval = time::interval(LOCAL_MOUSE_POLL);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        let sampled: Result<anyhow::Result<Point>, task::JoinError> =
            task::spawn_blocking(hypr::cursor).await;
        let at: Point = match sampled {
            Ok(Ok(at)) => at,
            Ok(Err(err)) => {
                eprintln!("cursor sample: {err:#}");
                continue;
            }
            Err(err) => {
                eprintln!("cursor sample: {err}");
                continue;
            }
        };
        let input: DeskInput = DeskInput::CursorSampled {
            at,
            at_ms: clock.now_ms(),
        };
        if inbox.send(DeskMessage::Input(input)).await.is_err() {
            return;
        }
    }
}

async fn run(program: &str, args: &[&str]) {
    match Command::new(program).args(args).output().await {
        Ok(output) if output.status.success() => {}
        Ok(output) => eprintln!(
            "{program} {args:?}: {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(err) => eprintln!("{program} {args:?}: {err}"),
    }
}

pub async fn pointer_theme() -> String {
    let theme: String = match Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "cursor-theme"])
        .output()
        .await
    {
        Ok(output) => String::from_utf8_lossy(&output.stdout)
            .trim()
            .trim_matches('\'')
            .to_owned(),
        Err(err) => {
            eprintln!("gsettings get cursor-theme: {err}");
            String::new()
        }
    };
    let index: std::path::PathBuf = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
        .join(".icons")
        .join(&theme)
        .join("index.theme");
    match std::fs::read_to_string(&index) {
        Ok(text) => inherited_theme(&text).unwrap_or(theme),
        Err(_) => theme,
    }
}

fn inherited_theme(index: &str) -> Option<String> {
    index
        .lines()
        .find_map(|line: &str| line.strip_prefix("Inherits="))
        .and_then(|list: &str| list.split(',').next())
        .map(|first: &str| first.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_inherits_line_keeps_the_theme() {
        assert_eq!(inherited_theme("[Icon Theme]\nName=x\n"), None);
    }

    #[test]
    fn one_inherited_theme() {
        assert_eq!(
            inherited_theme("[Icon Theme]\nInherits=Adwaita\n"),
            Some("Adwaita".to_owned())
        );
    }

    #[test]
    fn many_inherited_themes_take_the_first() {
        assert_eq!(
            inherited_theme("Inherits= Bibata-Modern-Classic , hicolor\n"),
            Some("Bibata-Modern-Classic".to_owned())
        );
    }
}
