mod cursor;
mod lane;
mod plan;
mod screencopy;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use axum::body::Bytes;
use phonepad_core::geometry::Size;
use phonepad_core::stream::UnitKind;
use tokio::sync::{broadcast, watch};

use self::screencopy::run;

#[derive(Clone, Debug)]
pub struct Unit {
    pub kind: UnitKind,
    pub message: Bytes,
}

#[derive(Clone, Debug, Default)]
pub struct KeyframeRequest(Arc<AtomicBool>);

impl KeyframeRequest {
    pub fn request(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn pending(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    fn take(&self) -> bool {
        self.0.swap(false, Ordering::Relaxed)
    }
}

#[derive(Clone)]
pub struct Feed {
    pub render_node: PathBuf,
    pub view: watch::Receiver<Option<Size>>,
    pub units: broadcast::Sender<Unit>,
    pub keyframe: KeyframeRequest,
}

pub struct Capture {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
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
    pub fn start(feed: Feed, on_exit: Box<dyn FnOnce() + Send>) -> std::io::Result<Capture> {
        let stop: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
        let stopped: Arc<AtomicBool> = Arc::clone(&stop);
        let thread: JoinHandle<()> =
            thread::Builder::new()
                .name("capture".to_owned())
                .spawn(move || {
                    let _notice: ExitNotice = ExitNotice(Some(on_exit));
                    if let Err(err) = run(&stopped, feed) {
                        eprintln!("capture: {err:#}");
                    }
                })?;
        Ok(Capture { stop, thread })
    }

    pub fn stop(self) -> JoinHandle<()> {
        self.stop.store(true, Ordering::Relaxed);
        self.thread
    }
}

#[cfg(test)]
mod tests {
    use super::KeyframeRequest;

    #[test]
    fn no_request_is_not_pending_and_takes_nothing() {
        let keyframe: KeyframeRequest = KeyframeRequest::default();
        assert_eq!([keyframe.pending(), keyframe.take()], [false, false]);
    }

    #[test]
    fn one_request_is_pending_until_taken_once() {
        let keyframe: KeyframeRequest = KeyframeRequest::default();
        keyframe.request();
        assert_eq!(
            [
                keyframe.pending(),
                keyframe.take(),
                keyframe.pending(),
                keyframe.take()
            ],
            [true, true, false, false]
        );
    }

    #[test]
    fn many_requests_from_many_viewers_coalesce_into_one_keyframe() {
        let keyframe: KeyframeRequest = KeyframeRequest::default();
        let other_viewer: KeyframeRequest = keyframe.clone();
        keyframe.request();
        other_viewer.request();
        keyframe.request();
        assert_eq!([keyframe.take(), keyframe.take()], [true, false]);
    }
}
