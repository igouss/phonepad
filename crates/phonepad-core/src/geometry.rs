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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transform {
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

impl Transform {
    pub fn from_wayland(code: u8) -> Option<Transform> {
        match code {
            0 => Some(Transform::Normal),
            1 => Some(Transform::Rotate90),
            2 => Some(Transform::Rotate180),
            3 => Some(Transform::Rotate270),
            4 => Some(Transform::Flipped),
            5 => Some(Transform::Flipped90),
            6 => Some(Transform::Flipped180),
            7 => Some(Transform::Flipped270),
            _ => None,
        }
    }

    pub fn wayland(self) -> u8 {
        match self {
            Transform::Normal => 0,
            Transform::Rotate90 => 1,
            Transform::Rotate180 => 2,
            Transform::Rotate270 => 3,
            Transform::Flipped => 4,
            Transform::Flipped90 => 5,
            Transform::Flipped180 => 6,
            Transform::Flipped270 => 7,
        }
    }

    pub fn swaps_axes(self) -> bool {
        match self {
            Transform::Rotate90
            | Transform::Rotate270
            | Transform::Flipped90
            | Transform::Flipped270 => true,
            Transform::Normal
            | Transform::Rotate180
            | Transform::Flipped
            | Transform::Flipped180 => false,
        }
    }

    pub fn oriented(self, size: Size) -> Size {
        if self.swaps_axes() {
            Size {
                w: size.h,
                h: size.w,
            }
        } else {
            size
        }
    }

    fn apply(self, crop: Crop, space: Size) -> Crop {
        let width: u32 = space.w.get();
        let height: u32 = space.h.get();
        let right: u32 = width.saturating_sub(crop.x + crop.w.get());
        let below: u32 = height.saturating_sub(crop.y + crop.h.get());
        let (x, y, swapped): (u32, u32, bool) = match self {
            Transform::Normal => (crop.x, crop.y, false),
            Transform::Rotate90 => (below, crop.x, true),
            Transform::Rotate180 => (right, below, false),
            Transform::Rotate270 => (crop.y, right, true),
            Transform::Flipped => (right, crop.y, false),
            Transform::Flipped90 => (crop.y, crop.x, true),
            Transform::Flipped180 => (crop.x, below, false),
            Transform::Flipped270 => (below, right, true),
        };
        let (w, h): (NonZeroU32, NonZeroU32) = if swapped {
            (crop.h, crop.w)
        } else {
            (crop.w, crop.h)
        };
        Crop { x, y, w, h }
    }

    fn inverse(self) -> Transform {
        match self {
            Transform::Rotate90 => Transform::Rotate270,
            Transform::Rotate270 => Transform::Rotate90,
            Transform::Normal
            | Transform::Rotate180
            | Transform::Flipped
            | Transform::Flipped90
            | Transform::Flipped180
            | Transform::Flipped270 => self,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub w: NonZeroU32,
    pub h: NonZeroU32,
}

impl Crop {
    pub fn overlaps(&self, other: Crop) -> bool {
        let spans = |a: u32, a_len: NonZeroU32, b: u32, b_len: NonZeroU32| -> bool {
            u64::from(a) < u64::from(b) + u64::from(b_len.get())
                && u64::from(b) < u64::from(a) + u64::from(a_len.get())
        };
        spans(self.x, self.w, other.x, other.w) && spans(self.y, self.h, other.y, other.h)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Monitor {
    pub name: String,
    pub origin: Point,
    pub size: Size,
    pub pixels: Size,
    pub transform: Transform,
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
        let transform: Transform = Transform::from_wayland(transform)?;
        let w: NonZeroU32 = logical(pixel_w, scale)?;
        let h: NonZeroU32 = logical(pixel_h, scale)?;
        let pixels: Size = Size {
            w: NonZeroU32::new(pixel_w)?,
            h: NonZeroU32::new(pixel_h)?,
        };
        Some(Monitor {
            name,
            origin,
            size: transform.oriented(Size { w, h }),
            pixels,
            transform,
        })
    }

    pub fn buffer_crop(&self, local: Point, size: Size) -> Option<Crop> {
        let out: Size = self.transform.oriented(self.pixels);
        let span = |start: i32, len: NonZeroU32, logical: NonZeroU32, pixels: NonZeroU32| {
            let start: u64 = u64::try_from(start).ok()?;
            let end: u64 = start + u64::from(len.get());
            let to_pixels = |at: u64| at * u64::from(pixels.get()) / u64::from(logical.get());
            let first: u32 = u32::try_from(to_pixels(start)).ok()?;
            let last: u32 = u32::try_from(to_pixels(end).min(u64::from(pixels.get()))).ok()?;
            Some((first, NonZeroU32::new(last.checked_sub(first)?)?))
        };
        let (x, w): (u32, NonZeroU32) = span(local.x, size.w, self.size.w, out.w)?;
        let (y, h): (u32, NonZeroU32) = span(local.y, size.h, self.size.h, out.h)?;
        Some(self.transform.inverse().apply(Crop { x, y, w, h }, out))
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
