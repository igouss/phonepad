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
mod tests {
    use super::*;

    fn nz(v: u32) -> NonZeroU32 {
        NonZeroU32::new(v).unwrap()
    }

    fn size(w: u32, h: u32) -> Size {
        Size { w: nz(w), h: nz(h) }
    }

    fn monitor(name: &str, x: i32, y: i32, w: u32, h: u32) -> Monitor {
        Monitor {
            name: name.to_string(),
            origin: Point { x, y },
            size: size(w, h),
        }
    }

    fn mode(pixel_w: u32, pixel_h: u32, scale: f64, transform: u8) -> Option<Monitor> {
        Monitor::from_mode(
            "DP-1".to_string(),
            Point { x: 10, y: 20 },
            pixel_w,
            pixel_h,
            scale,
            transform,
        )
    }

    fn region(
        output: &str,
        global: (i32, i32),
        local: (i32, i32),
        w: u32,
        h: u32,
    ) -> Option<Region> {
        Some(Region {
            output: output.to_string(),
            global: Point {
                x: global.0,
                y: global.1,
            },
            local: Point {
                x: local.0,
                y: local.1,
            },
            size: size(w, h),
        })
    }

    fn two_monitors() -> Vec<Monitor> {
        vec![
            monitor("A", 0, 0, 1920, 1080),
            monitor("B", 1920, 200, 2560, 1440),
        ]
    }

    const VIEW: Size = Size {
        w: NonZeroU32::new(640).unwrap(),
        h: NonZeroU32::new(480).unwrap(),
    };

    #[test]
    fn unit_scale_keeps_pixel_size() {
        assert_eq!(
            mode(1920, 1080, 1.0, 0),
            Some(monitor("DP-1", 10, 20, 1920, 1080))
        );
    }

    #[test]
    fn fractional_scale_divides_and_rounds() {
        assert_eq!(
            mode(2880, 1921, 1.5, 0),
            Some(monitor("DP-1", 10, 20, 1920, 1281))
        );
    }

    #[test]
    fn rounding_ties_go_to_even() {
        assert_eq!(mode(5, 7, 2.0, 0), Some(monitor("DP-1", 10, 20, 2, 4)));
    }

    #[test]
    fn transform_one_swaps_axes() {
        assert_eq!(
            mode(1920, 1080, 1.0, 1),
            Some(monitor("DP-1", 10, 20, 1080, 1920))
        );
    }

    #[test]
    fn transform_two_keeps_axes() {
        assert_eq!(
            mode(1920, 1080, 1.0, 2),
            Some(monitor("DP-1", 10, 20, 1920, 1080))
        );
    }

    #[test]
    fn transform_seven_swaps_axes() {
        assert_eq!(
            mode(1920, 1080, 1.0, 7),
            Some(monitor("DP-1", 10, 20, 1080, 1920))
        );
    }

    #[test]
    fn transform_eight_is_rejected() {
        assert_eq!(mode(1920, 1080, 1.0, 8), None);
    }

    #[test]
    fn zero_scale_is_rejected() {
        assert_eq!(mode(1920, 1080, 0.0, 0), None);
    }

    #[test]
    fn negative_scale_is_rejected() {
        assert_eq!(mode(1920, 1080, -1.0, 0), None);
    }

    #[test]
    fn nan_scale_is_rejected() {
        assert_eq!(mode(1920, 1080, f64::NAN, 0), None);
    }

    #[test]
    fn infinite_scale_is_rejected() {
        assert_eq!(mode(1920, 1080, f64::INFINITY, 0), None);
    }

    #[test]
    fn zero_width_is_rejected() {
        assert_eq!(mode(0, 1080, 1.0, 0), None);
    }

    #[test]
    fn zero_height_is_rejected() {
        assert_eq!(mode(1920, 0, 1.0, 0), None);
    }

    #[test]
    fn width_rounding_to_zero_is_rejected() {
        assert_eq!(mode(1, 1080, 3.0, 0), None);
    }

    #[test]
    fn largest_representable_width_is_kept() {
        assert_eq!(
            mode(u32::MAX, 1, 1.0, 0),
            Some(monitor("DP-1", 10, 20, u32::MAX, 1))
        );
    }

    #[test]
    fn width_beyond_u32_is_rejected() {
        assert_eq!(mode(u32::MAX, 1, 0.5, 0), None);
    }

    #[test]
    fn no_monitors_give_no_region() {
        assert_eq!(region_around(Point { x: 0, y: 0 }, VIEW, &[]), None);
    }

    #[test]
    fn region_centres_on_cursor() {
        let monitors: [Monitor; 1] = [monitor("A", 0, 0, 1920, 1080)];
        assert_eq!(
            region_around(Point { x: 1000, y: 500 }, VIEW, &monitors),
            region("A", (680, 260), (680, 260), 640, 480)
        );
    }

    #[test]
    fn odd_view_rounds_down_to_an_even_region() {
        let monitors: [Monitor; 1] = [monitor("A", 0, 0, 1920, 1080)];
        assert_eq!(
            region_around(Point { x: 1000, y: 500 }, size(641, 481), &monitors),
            region("A", (680, 260), (680, 260), 640, 480)
        );
    }

    #[test]
    fn even_view_is_kept() {
        let monitors: [Monitor; 1] = [monitor("A", 0, 0, 1920, 1080)];
        assert_eq!(
            region_around(Point { x: 1000, y: 500 }, size(642, 482), &monitors),
            region("A", (679, 259), (679, 259), 642, 482)
        );
    }

    #[test]
    fn view_of_one_grows_to_two() {
        let monitors: [Monitor; 1] = [monitor("A", 0, 0, 1920, 1080)];
        assert_eq!(
            region_around(Point { x: 1000, y: 500 }, size(1, 1), &monitors),
            region("A", (999, 499), (999, 499), 2, 2)
        );
    }

    #[test]
    fn odd_monitor_smaller_than_view_gives_an_even_region() {
        let monitors: [Monitor; 1] = [monitor("A", 0, 0, 1921, 1281)];
        assert_eq!(
            region_around(Point { x: 1000, y: 500 }, size(3000, 3000), &monitors),
            region("A", (1, 0), (1, 0), 1920, 1280)
        );
    }

    #[test]
    fn monitor_one_pixel_wide_gives_no_region() {
        let monitors: [Monitor; 1] = [monitor("A", 0, 0, 1, 1080)];
        assert_eq!(region_around(Point { x: 0, y: 500 }, VIEW, &monitors), None);
    }

    #[test]
    fn monitor_one_pixel_high_gives_no_region() {
        let monitors: [Monitor; 1] = [monitor("A", 0, 0, 1920, 1)];
        assert_eq!(
            region_around(Point { x: 1000, y: 0 }, VIEW, &monitors),
            None
        );
    }

    #[test]
    fn region_clamps_to_monitor_start() {
        let monitors: [Monitor; 1] = [monitor("A", 0, 0, 1920, 1080)];
        assert_eq!(
            region_around(Point { x: 10, y: 10 }, VIEW, &monitors),
            region("A", (0, 0), (0, 0), 640, 480)
        );
    }

    #[test]
    fn region_clamps_to_monitor_end() {
        let monitors: [Monitor; 1] = [monitor("A", 0, 0, 1920, 1080)];
        assert_eq!(
            region_around(Point { x: 1915, y: 1075 }, VIEW, &monitors),
            region("A", (1280, 600), (1280, 600), 640, 480)
        );
    }

    #[test]
    fn region_shrinks_each_axis_to_monitor() {
        let monitors: [Monitor; 1] = [monitor("A", 0, 0, 1920, 1080)];
        assert_eq!(
            region_around(Point { x: 1000, y: 500 }, size(3000, 100), &monitors),
            region("A", (0, 450), (0, 450), 1920, 100)
        );
    }

    #[test]
    fn region_uses_monitor_holding_cursor() {
        assert_eq!(
            region_around(Point { x: 3000, y: 900 }, VIEW, &two_monitors()),
            region("B", (2680, 660), (760, 460), 640, 480)
        );
    }

    #[test]
    fn region_keeps_last_column_on_left_monitor() {
        assert_eq!(
            region_around(Point { x: 1919, y: 500 }, VIEW, &two_monitors()),
            region("A", (1280, 260), (1280, 260), 640, 480)
        );
    }

    #[test]
    fn region_moves_to_right_monitor_at_its_origin() {
        assert_eq!(
            region_around(Point { x: 1920, y: 200 }, VIEW, &two_monitors()),
            region("B", (1920, 200), (0, 0), 640, 480)
        );
    }

    #[test]
    fn region_excludes_row_below_monitor() {
        assert_eq!(
            region_around(Point { x: 1920, y: 1640 }, VIEW, &two_monitors()),
            region("A", (1280, 600), (1280, 600), 640, 480)
        );
    }

    #[test]
    fn region_falls_back_to_first_monitor() {
        assert_eq!(
            region_around(Point { x: -500, y: -500 }, VIEW, &two_monitors()),
            region("A", (0, 0), (0, 0), 640, 480)
        );
    }
}
