use std::num::NonZeroU32;
use std::time::Duration;

use phonepad_core::geometry::{Crop, Monitor, Point, Region, Size, Transform, window_around};

pub(super) const MIN_ENCODE_INTERVAL: Duration = Duration::from_micros(16_667);
pub(super) const MIN_KEYFRAME_INTERVAL: Duration = Duration::from_millis(500);
const NANOS_PER_SECOND: u64 = 1_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Pixels {
    pub(super) width: u32,
    pub(super) height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Shape {
    pub(super) output: String,
    pub(super) source: Pixels,
    pub(super) window: Pixels,
    pub(super) transform: Transform,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Shot {
    pub(super) crop: Crop,
    pub(super) window: Point,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Target {
    pub(super) shape: Shape,
    pub(super) shot: Shot,
    pub(super) window: Region,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Buffer {
    pub(super) fourcc: u32,
    pub(super) modifier: u64,
    pub(super) width: u32,
    pub(super) height: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Constraints {
    pub(super) size: Option<(u32, u32)>,
    pub(super) dmabuf: Vec<(u32, Vec<u64>)>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(super) enum Misfit {
    #[error("the compositor gave no buffer size")]
    NoSize,
    #[error("the compositor wants {wanted:?} pixel buffers, the encoder surfaces are {have:?}")]
    Size {
        wanted: (u32, u32),
        have: (u32, u32),
    },
    #[error(
        "the compositor takes no dmabuf of format {fourcc:#x} with modifier {modifier:#x}; it offers {offered:x?}"
    )]
    Format {
        fourcc: u32,
        modifier: u64,
        offered: Vec<(u32, Vec<u64>)>,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Slot<F> {
    Free,
    Capturing {
        frame: F,
        session: usize,
        damage: Vec<Crop>,
    },
    Held {
        stamp: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Shown {
    pub(super) shot: Shot,
    pub(super) stamp: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Schedule {
    Idle,
    Busy,
    EncodeNow,
    WaitFor(Duration),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Pending {
    pub(super) changed: bool,
    pub(super) keyframe: bool,
    pub(super) encoder_busy: bool,
}

fn pixels(size: Size) -> Pixels {
    Pixels {
        width: size.w.get(),
        height: size.h.get(),
    }
}

pub(super) fn target(
    cursor: Point,
    view: Size,
    monitors: &[Monitor],
    kept: Option<&Region>,
) -> Option<Target> {
    let window: Region = window_around(cursor, view, monitors, kept)?;
    let monitor: &Monitor = monitors
        .iter()
        .find(|m: &&Monitor| m.name == window.output)?;
    Some(Target {
        shape: Shape {
            output: window.output.clone(),
            source: pixels(monitor.pixels),
            window: pixels(monitor.transform.oriented(window.size)),
            transform: monitor.transform,
        },
        shot: Shot {
            crop: monitor.buffer_crop(window.local, window.size)?,
            window: window.global,
        },
        window,
    })
}

pub(super) fn keyframe_due(requested: bool, since_keyframe: Option<Duration>) -> bool {
    requested && since_keyframe.is_none_or(|elapsed: Duration| elapsed >= MIN_KEYFRAME_INTERVAL)
}

pub(super) fn fits(constraints: &Constraints, buffer: Buffer) -> Result<(), Misfit> {
    let size: (u32, u32) = constraints.size.ok_or(Misfit::NoSize)?;
    let have: (u32, u32) = (buffer.width, buffer.height);
    if size != have {
        return Err(Misfit::Size { wanted: size, have });
    }
    if constraints
        .dmabuf
        .iter()
        .any(|(fourcc, modifiers): &(u32, Vec<u64>)| {
            *fourcc == buffer.fourcc && modifiers.contains(&buffer.modifier)
        })
    {
        Ok(())
    } else {
        Err(Misfit::Format {
            fourcc: buffer.fourcc,
            modifier: buffer.modifier,
            offered: constraints.dmabuf.clone(),
        })
    }
}

pub(super) fn held<F>(slots: &[Slot<F>]) -> Option<(usize, u64)> {
    slots
        .iter()
        .enumerate()
        .find_map(|(index, slot): (usize, &Slot<F>)| match slot {
            Slot::Held { stamp } => Some((index, *stamp)),
            Slot::Free | Slot::Capturing { .. } => None,
        })
}

pub(super) fn to_issue<F>(slots: &[Slot<F>], pinned: &[bool], in_flight: usize) -> Vec<usize> {
    let capturing: usize = slots
        .iter()
        .filter(|slot: &&Slot<F>| match slot {
            Slot::Capturing { .. } => true,
            Slot::Free | Slot::Held { .. } => false,
        })
        .count();
    slots
        .iter()
        .enumerate()
        .filter_map(|(index, slot): (usize, &Slot<F>)| match slot {
            Slot::Free if !pinned.get(index).copied().unwrap_or(true) => Some(index),
            Slot::Free | Slot::Capturing { .. } | Slot::Held { .. } => None,
        })
        .take(in_flight.saturating_sub(capturing))
        .collect()
}

pub(super) fn arrive<F>(slots: &mut [Slot<F>], index: usize, stamp: u64) {
    match held(slots) {
        Some((_, held)) if held >= stamp => slots[index] = Slot::Free,
        Some((previous, _)) => {
            slots[previous] = Slot::Free;
            slots[index] = Slot::Held { stamp };
        }
        None => slots[index] = Slot::Held { stamp },
    }
}

pub(super) fn damage(x: i32, y: i32, width: i32, height: i32) -> Option<Crop> {
    let clip = |start: i32, len: i32| -> Option<(u32, NonZeroU32)> {
        let first: i64 = i64::from(start).max(0);
        let end: i64 = i64::from(start) + i64::from(len);
        Some((
            u32::try_from(first).ok()?,
            NonZeroU32::new(u32::try_from(end - first).ok()?)?,
        ))
    };
    let (x, w): (u32, NonZeroU32) = clip(x, width)?;
    let (y, h): (u32, NonZeroU32) = clip(y, height)?;
    Some(Crop { x, y, w, h })
}

pub(super) fn damaged<F>(slots: &mut [Slot<F>], index: usize, rect: Crop) {
    match slots.get_mut(index) {
        Some(Slot::Capturing { damage, .. }) => damage.push(rect),
        Some(Slot::Free | Slot::Held { .. }) | None => {}
    }
}

pub(super) fn changes_shown<F>(slot: &Slot<F>, stamp: u64, shown: Option<Shown>) -> bool {
    match (slot, shown) {
        (Slot::Capturing { .. }, None) => true,
        (Slot::Capturing { damage, .. }, Some(shown)) => {
            stamp > shown.stamp
                && damage
                    .iter()
                    .any(|rect: &Crop| rect.overlaps(shown.shot.crop))
        }
        (Slot::Free | Slot::Held { .. }, Some(_) | None) => false,
    }
}

pub(super) fn stamp(seconds_high: u32, seconds_low: u32, nanos: u32) -> u64 {
    ((u64::from(seconds_high) << 32) + u64::from(seconds_low))
        .saturating_mul(NANOS_PER_SECOND)
        .saturating_add(u64::from(nanos))
}

pub(super) fn schedule(
    pending: Pending,
    shown: Option<Shot>,
    wanted: Shot,
    since_last: Option<Duration>,
) -> Schedule {
    let changed: bool = pending.changed || pending.keyframe || shown != Some(wanted);
    match (changed, pending.encoder_busy, since_last) {
        (false, _, _) => Schedule::Idle,
        (true, true, _) => Schedule::Busy,
        (true, false, None) => Schedule::EncodeNow,
        (true, false, Some(elapsed)) => match MIN_ENCODE_INTERVAL.checked_sub(elapsed) {
            Some(left) if !left.is_zero() => Schedule::WaitFor(left),
            Some(_) | None => Schedule::EncodeNow,
        },
    }
}

#[cfg(test)]
mod tests;
