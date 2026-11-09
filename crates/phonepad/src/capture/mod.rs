mod cursor;
mod format;
mod screencopy;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use phonepad_core::geometry::{Point, Size};
use tokio::sync::{broadcast, watch};

use self::cursor::sample_cursor;
use self::screencopy::run;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelOrder {
    Bgra,
    Rgba,
}

#[derive(Debug)]
pub struct Frame {
    pub origin: Point,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub order: ChannelOrder,
    pub y_invert: bool,
    pub pixels: Vec<u8>,
}

pub type Frames = broadcast::Sender<Arc<Frame>>;

pub struct Capture {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
    sampler: JoinHandle<()>,
}

struct ExitNotice(Option<Box<dyn FnOnce() + Send>>);

impl Drop for ExitNotice {
    fn drop(&mut self) {
        if let Some(notify) = self.0.take() {
            notify();
        }
    }
}

impl Capture {
    pub fn start(
        view: watch::Receiver<Option<Size>>,
        cursor: watch::Sender<Option<Point>>,
        frames: Frames,
        on_exit: Box<dyn FnOnce() + Send>,
    ) -> Capture {
        let stop: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
        let stopped: Arc<AtomicBool> = Arc::clone(&stop);
        let sampling: Arc<AtomicBool> = Arc::clone(&stop);
        let cursor_seen: watch::Receiver<Option<Point>> = cursor.subscribe();
        let sampler: JoinHandle<()> = thread::spawn(move || sample_cursor(&sampling, &cursor));
        let thread: JoinHandle<()> = thread::spawn(move || {
            let _notice: ExitNotice = ExitNotice(Some(on_exit));
            if let Err(err) = run(&stopped, view, cursor_seen, frames) {
                eprintln!("capture: {err:#}");
            }
        });
        Capture {
            stop,
            thread,
            sampler,
        }
    }

    pub fn stop(self) -> [JoinHandle<()>; 2] {
        self.stop.store(true, Ordering::Relaxed);
        [self.thread, self.sampler]
    }
}
