use std::num::NonZeroU32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub w: NonZeroU32,
    pub h: NonZeroU32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Monitor {
    pub name: String,
    pub origin: Point,
    pub size: Size,
}

impl Monitor {
    pub fn from_mode(
        name: String,
        origin: Point,
        pixel_w: u32,
        pixel_h: u32,
        scale: f64,
        transform: u8,
    ) -> Option<Monitor> {
        if transform > 7 {
            return None;
        }
        let w: NonZeroU32 = logical(pixel_w, scale)?;
        let h: NonZeroU32 = logical(pixel_h, scale)?;
        let rotated: bool = transform % 2 == 1;
        let size: Size = if rotated {
            Size { w: h, h: w }
        } else {
            Size { w, h }
        };
        Some(Monitor { name, origin, size })
    }

    fn contains(&self, p: Point) -> bool {
        within(p.x, self.origin.x, self.size.w) && within(p.y, self.origin.y, self.size.h)
    }
}

fn logical(pixels: u32, scale: f64) -> Option<NonZeroU32> {
    let rounded: f64 = (f64::from(pixels) / scale).round_ties_even();
    if (1.0..=f64::from(u32::MAX)).contains(&rounded) {
        NonZeroU32::new(rounded as u32)
    } else {
        None
    }
}

fn within(c: i32, start: i32, len: NonZeroU32) -> bool {
    let c: i64 = i64::from(c);
    let start: i64 = i64::from(start);
    start <= c && c < start + i64::from(len.get())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub output: String,
    pub global: Point,
    pub local: Point,
    pub size: Size,
}

pub fn region_around(cursor: Point, view: Size, monitors: &[Monitor]) -> Option<Region> {
    let first: &Monitor = monitors.first()?;
    let mon: &Monitor = monitors
        .iter()
        .find(|m: &&Monitor| m.contains(cursor))
        .unwrap_or(first);
    let size: Size = Size {
        w: even_within(view.w, mon.size.w)?,
        h: even_within(view.h, mon.size.h)?,
    };
    let gx: i64 = start_around(cursor.x, size.w, mon.origin.x, mon.size.w);
    let gy: i64 = start_around(cursor.y, size.h, mon.origin.y, mon.size.h);
    let global: Point = Point {
        x: i32::try_from(gx).ok()?,
        y: i32::try_from(gy).ok()?,
    };
    let local: Point = Point {
        x: i32::try_from(gx - i64::from(mon.origin.x)).ok()?,
        y: i32::try_from(gy - i64::from(mon.origin.y)).ok()?,
    };
    Some(Region {
        output: mon.name.clone(),
        global,
        local,
        size,
    })
}

pub const WINDOW_MARGIN_PERCENT: u32 = 25;

pub fn window_around(
    cursor: Point,
    view: Size,
    monitors: &[Monitor],
    kept: Option<&Region>,
) -> Option<Region> {
    let fresh: Region = region_around(cursor, grown(view), monitors)?;
    let seen: Region = region_around(cursor, view, monitors)?;
    match kept {
        Some(window) if window.size == fresh.size && holds(window, &seen) => Some(window.clone()),
        Some(_) | None => Some(fresh),
    }
}

fn grown(view: Size) -> Size {
    let side = |len: NonZeroU32| -> NonZeroU32 {
        let margin: u32 = len.get() / 100 * WINDOW_MARGIN_PERCENT * 2
            + len.get() % 100 * WINDOW_MARGIN_PERCENT * 2 / 100;
        len.saturating_add(margin)
    };
    Size {
        w: side(view.w),
        h: side(view.h),
    }
}

fn holds(outer: &Region, inner: &Region) -> bool {
    let spans = |start: i32, len: NonZeroU32, inner_start: i32, inner_len: NonZeroU32| -> bool {
        let start: i64 = i64::from(start);
        let inner_start: i64 = i64::from(inner_start);
        start <= inner_start
            && inner_start + i64::from(inner_len.get()) <= start + i64::from(len.get())
    };
    outer.output == inner.output
        && spans(outer.global.x, outer.size.w, inner.global.x, inner.size.w)
        && spans(outer.global.y, outer.size.h, inner.global.y, inner.size.h)
}

fn even_within(wanted: NonZeroU32, limit: NonZeroU32) -> Option<NonZeroU32> {
    NonZeroU32::new(wanted.get().max(2).min(limit.get()) & !1)
}

fn start_around(c: i32, len: NonZeroU32, mon_start: i32, mon_len: NonZeroU32) -> i64 {
    let len: i64 = i64::from(len.get());
    let lo: i64 = i64::from(mon_start);
    let hi: i64 = lo + i64::from(mon_len.get()) - len;
    (i64::from(c) - len / 2).clamp(lo, hi)
}

#[cfg(test)]
mod tests;
