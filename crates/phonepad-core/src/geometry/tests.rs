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

fn region(output: &str, global: (i32, i32), local: (i32, i32), w: u32, h: u32) -> Option<Region> {
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

const SMALL: Size = Size {
    w: NonZeroU32::new(400).unwrap(),
    h: NonZeroU32::new(200).unwrap(),
};

fn wide() -> Vec<Monitor> {
    vec![monitor("A", 0, 0, 2560, 1440)]
}

#[test]
fn a_view_grows_by_a_quarter_on_each_side() {
    assert_eq!(grown(SMALL), size(600, 300));
}

#[test]
fn growth_keeps_the_remainder_of_a_hundred() {
    assert_eq!(grown(size(150, 1)), size(225, 1));
}

#[test]
fn growth_saturates_instead_of_wrapping() {
    assert_eq!(grown(size(u32::MAX, 2)), size(u32::MAX, 3));
}

#[test]
fn no_window_was_kept_so_one_is_centred_on_the_cursor() {
    assert_eq!(
        window_around(Point { x: 1000, y: 700 }, SMALL, &wide(), None),
        region("A", (700, 550), (700, 550), 600, 300)
    );
}

#[test]
fn a_kept_window_that_still_holds_the_view_is_kept() {
    let kept: Option<Region> = region("A", (700, 550), (700, 550), 600, 300);
    assert_eq!(
        window_around(Point { x: 1090, y: 740 }, SMALL, &wide(), kept.as_ref()),
        kept
    );
}

#[test]
fn a_view_touching_the_kept_window_edge_keeps_it() {
    let kept: Option<Region> = region("A", (700, 550), (700, 550), 600, 300);
    assert_eq!(
        window_around(Point { x: 1100, y: 750 }, SMALL, &wide(), kept.as_ref()),
        kept
    );
}

#[test]
fn a_view_one_pixel_past_the_right_edge_recentres() {
    let kept: Option<Region> = region("A", (700, 550), (700, 550), 600, 300);
    assert_eq!(
        window_around(Point { x: 1101, y: 700 }, SMALL, &wide(), kept.as_ref()),
        region("A", (801, 550), (801, 550), 600, 300)
    );
}

#[test]
fn a_view_one_pixel_past_the_left_edge_recentres() {
    let kept: Option<Region> = region("A", (700, 550), (700, 550), 600, 300);
    assert_eq!(
        window_around(Point { x: 899, y: 700 }, SMALL, &wide(), kept.as_ref()),
        region("A", (599, 550), (599, 550), 600, 300)
    );
}

#[test]
fn a_view_one_pixel_past_the_top_edge_recentres() {
    let kept: Option<Region> = region("A", (700, 550), (700, 550), 600, 300);
    assert_eq!(
        window_around(Point { x: 1000, y: 649 }, SMALL, &wide(), kept.as_ref()),
        region("A", (700, 499), (700, 499), 600, 300)
    );
}

#[test]
fn a_view_one_pixel_past_the_bottom_edge_recentres() {
    let kept: Option<Region> = region("A", (700, 550), (700, 550), 600, 300);
    assert_eq!(
        window_around(Point { x: 1000, y: 751 }, SMALL, &wide(), kept.as_ref()),
        region("A", (700, 601), (700, 601), 600, 300)
    );
}

#[test]
fn a_kept_window_of_another_size_is_replaced() {
    let kept: Option<Region> = region("A", (700, 550), (700, 550), 602, 300);
    assert_eq!(
        window_around(Point { x: 1000, y: 700 }, SMALL, &wide(), kept.as_ref()),
        region("A", (700, 550), (700, 550), 600, 300)
    );
}

#[test]
fn a_kept_window_on_another_monitor_is_replaced() {
    let kept: Option<Region> = region("A", (1280, 600), (1280, 600), 600, 300);
    assert_eq!(
        window_around(
            Point { x: 3000, y: 900 },
            SMALL,
            &two_monitors(),
            kept.as_ref()
        ),
        region("B", (2700, 750), (780, 550), 600, 300)
    );
}

#[test]
fn a_window_at_the_monitor_corner_holds_the_clamped_view() {
    let kept: Option<Region> = region("A", (0, 0), (0, 0), 600, 300);
    assert_eq!(
        window_around(Point { x: 5, y: 5 }, SMALL, &wide(), kept.as_ref()),
        kept
    );
}

#[test]
fn a_view_larger_than_the_monitor_gives_the_whole_monitor() {
    assert_eq!(
        window_around(Point { x: 1000, y: 700 }, size(2400, 1300), &wide(), None),
        region("A", (0, 0), (0, 0), 2560, 1440)
    );
}

#[test]
fn no_monitor_gives_no_window() {
    assert_eq!(window_around(Point { x: 0, y: 0 }, SMALL, &[], None), None);
}

#[test]
fn region_falls_back_to_first_monitor() {
    assert_eq!(
        region_around(Point { x: -500, y: -500 }, VIEW, &two_monitors()),
        region("A", (0, 0), (0, 0), 640, 480)
    );
}
