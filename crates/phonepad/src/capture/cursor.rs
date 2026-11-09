use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use phonepad_core::geometry::Point;
use tokio::sync::watch;

use crate::hypr;

const CURSOR_SAMPLE_PERIOD: Duration = Duration::from_millis(5);

pub(super) fn sample_cursor(stop: &AtomicBool, cursor: &watch::Sender<Option<Point>>) {
    let mut failing: bool = false;
    while !stop.load(Ordering::Relaxed) {
        match hypr::cursor() {
            Ok(at) => {
                cursor.send_replace(Some(at));
                failing = false;
            }
            Err(err) => {
                if !failing {
                    eprintln!("cursor sample: {err:#}");
                }
                failing = true;
            }
        }
        thread::sleep(CURSOR_SAMPLE_PERIOD);
    }
}
