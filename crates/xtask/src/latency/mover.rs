use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, InputEvent, KeyCode, RelativeAxisCode, RelativeAxisEvent};
use std::f64::consts::TAU;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub const STEPS_PER_TURN: u32 = 240;
pub const STEP_PX: f64 = 6.0;
const TICK: Duration = Duration::from_micros(8_333);
const SETTLE: Duration = Duration::from_secs(1);

pub struct Mover {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<Result<(), String>>,
}

pub fn step(index: u32) -> (i32, i32) {
    let angle: f64 = f64::from(index) * TAU / f64::from(STEPS_PER_TURN);
    (
        (STEP_PX * angle.cos()).round() as i32,
        (STEP_PX * angle.sin()).round() as i32,
    )
}

fn device() -> Result<VirtualDevice, String> {
    let axes: AttributeSet<RelativeAxisCode> = [RelativeAxisCode::REL_X, RelativeAxisCode::REL_Y]
        .into_iter()
        .collect();
    let keys: AttributeSet<KeyCode> = std::iter::once(KeyCode::BTN_LEFT).collect();
    VirtualDevice::builder()
        .and_then(|builder| {
            builder
                .name("phonepad-latency-mover")
                .with_relative_axes(&axes)
        })
        .and_then(|builder| builder.with_keys(&keys))
        .and_then(|builder| builder.build())
        .map_err(|e| format!("uinput mover: {e}"))
}

fn circle(mut device: VirtualDevice, stop: &AtomicBool) -> Result<(), String> {
    thread::sleep(SETTLE);
    let mut index: u32 = 0;
    while !stop.load(Ordering::Relaxed) {
        let (dx, dy): (i32, i32) = step(index);
        let events: [InputEvent; 2] = [
            *RelativeAxisEvent::new(RelativeAxisCode::REL_X, dx),
            *RelativeAxisEvent::new(RelativeAxisCode::REL_Y, dy),
        ];
        device
            .emit(&events)
            .map_err(|e| format!("uinput mover: {e}"))?;
        index = (index + 1) % STEPS_PER_TURN;
        thread::sleep(TICK);
    }
    Ok(())
}

impl Mover {
    pub fn start() -> Result<Mover, String> {
        let device: VirtualDevice = device()?;
        let stop: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
        let stopped: Arc<AtomicBool> = Arc::clone(&stop);
        let thread: JoinHandle<Result<(), String>> =
            thread::spawn(move || circle(device, &stopped));
        Ok(Mover { stop, thread })
    }

    pub fn stop(self) -> Result<(), String> {
        self.stop.store(true, Ordering::Relaxed);
        self.thread
            .join()
            .map_err(|_| String::from("the mover panicked"))?
    }
}

#[cfg(test)]
mod tests {
    use super::{STEPS_PER_TURN, step};

    #[test]
    fn the_first_step_points_right() {
        assert_eq!(step(0), (6, 0));
    }

    #[test]
    fn a_quarter_turn_points_down() {
        assert_eq!(step(STEPS_PER_TURN / 4), (0, 6));
    }

    #[test]
    fn a_half_turn_points_left() {
        assert_eq!(step(STEPS_PER_TURN / 2), (-6, 0));
    }

    #[test]
    fn the_last_step_closes_the_circle() {
        assert_eq!(step(STEPS_PER_TURN - 1), (6, 0));
    }
}
