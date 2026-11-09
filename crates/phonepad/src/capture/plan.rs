use super::Frame;

pub(super) const IN_FLIGHT: usize = 4;
const INVISIBLE_CHANGE: u8 = 2;
const MOVING_IN_FLIGHT: usize = 2;
const STILL_IN_FLIGHT: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CopyMode {
    Immediate,
    OnDamage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct IssuePlan {
    pub(super) count: usize,
    pub(super) mode: CopyMode,
}

pub(super) fn plan_issue(
    region_moved: bool,
    in_flight: usize,
    recently_unchanged: bool,
) -> IssuePlan {
    match (region_moved, recently_unchanged) {
        (true, _) => IssuePlan {
            count: MOVING_IN_FLIGHT.saturating_sub(in_flight),
            mode: CopyMode::Immediate,
        },
        (false, true) => IssuePlan {
            count: 0,
            mode: CopyMode::OnDamage,
        },
        (false, false) => IssuePlan {
            count: STILL_IN_FLIGHT.saturating_sub(in_flight),
            mode: CopyMode::OnDamage,
        },
    }
}

pub(super) fn is_new_image(published: Option<&Frame>, captured: &Frame) -> bool {
    published.is_none_or(|last: &Frame| {
        last.origin != captured.origin
            || last.width != captured.width
            || last.height != captured.height
            || last.stride != captured.stride
            || last.order != captured.order
            || last.y_invert != captured.y_invert
            || (last.pixels != captured.pixels && visibly_differs(&last.pixels, &captured.pixels))
    })
}

fn visibly_differs(before: &[u8], after: &[u8]) -> bool {
    before.len() != after.len()
        || before
            .iter()
            .zip(after)
            .any(|(b, a): (&u8, &u8)| b.abs_diff(*a) > INVISIBLE_CHANGE)
}

#[cfg(test)]
mod tests {
    use phonepad_core::geometry::Point;

    use super::*;
    use crate::capture::ChannelOrder;
    fn frame(origin: Point, pixels: Vec<u8>) -> Frame {
        Frame {
            origin,
            width: 1,
            height: 1,
            stride: 4,
            order: ChannelOrder::Bgra,
            y_invert: false,
            pixels,
        }
    }

    const HERE: Point = Point { x: 10, y: 20 };
    const THERE: Point = Point { x: 11, y: 20 };

    #[test]
    fn first_capture_is_a_new_image() {
        assert!(is_new_image(None, &frame(HERE, vec![1, 2, 3, 4])));
    }

    #[test]
    fn identical_capture_is_not_a_new_image() {
        assert!(!is_new_image(
            Some(&frame(HERE, vec![1, 2, 3, 4])),
            &frame(HERE, vec![1, 2, 3, 4])
        ));
    }

    #[test]
    fn changed_pixels_are_a_new_image() {
        assert!(is_new_image(
            Some(&frame(HERE, vec![1, 2, 3, 4])),
            &frame(HERE, vec![1, 2, 3, 9])
        ));
    }

    #[test]
    fn change_within_the_invisible_margin_is_not_a_new_image() {
        assert!(!is_new_image(
            Some(&frame(HERE, vec![10, 20, 30, 40])),
            &frame(HERE, vec![12, 18, 30, 40])
        ));
    }

    #[test]
    fn change_just_beyond_the_invisible_margin_is_a_new_image() {
        assert!(is_new_image(
            Some(&frame(HERE, vec![10, 20, 30, 40])),
            &frame(HERE, vec![13, 20, 30, 40])
        ));
    }

    #[test]
    fn moved_region_with_same_pixels_is_a_new_image() {
        assert!(is_new_image(
            Some(&frame(HERE, vec![1, 2, 3, 4])),
            &frame(THERE, vec![1, 2, 3, 4])
        ));
    }

    #[test]
    fn moved_region_with_nothing_in_flight_issues_moving_count_immediately() {
        assert_eq!(
            plan_issue(true, 0, false),
            IssuePlan {
                count: MOVING_IN_FLIGHT,
                mode: CopyMode::Immediate
            }
        );
    }

    #[test]
    fn moved_region_tops_up_to_moving_count() {
        assert_eq!(
            plan_issue(true, 1, false),
            IssuePlan {
                count: MOVING_IN_FLIGHT - 1,
                mode: CopyMode::Immediate
            }
        );
    }

    #[test]
    fn moved_region_with_many_in_flight_issues_none() {
        assert_eq!(
            plan_issue(true, IN_FLIGHT, false),
            IssuePlan {
                count: 0,
                mode: CopyMode::Immediate
            }
        );
    }

    #[test]
    fn still_region_with_nothing_in_flight_waits_for_damage_once() {
        assert_eq!(
            plan_issue(false, 0, false),
            IssuePlan {
                count: STILL_IN_FLIGHT,
                mode: CopyMode::OnDamage
            }
        );
    }

    #[test]
    fn still_region_just_seen_unchanged_waits_before_rechecking() {
        assert_eq!(
            plan_issue(false, 0, true),
            IssuePlan {
                count: 0,
                mode: CopyMode::OnDamage
            }
        );
    }

    #[test]
    fn moved_region_ignores_the_quiet_recheck_delay() {
        assert_eq!(
            plan_issue(true, 0, true),
            IssuePlan {
                count: MOVING_IN_FLIGHT,
                mode: CopyMode::Immediate
            }
        );
    }

    #[test]
    fn still_region_with_one_in_flight_issues_none() {
        assert_eq!(
            plan_issue(false, 1, false),
            IssuePlan {
                count: 0,
                mode: CopyMode::OnDamage
            }
        );
    }
}
