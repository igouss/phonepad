use std::num::NonZeroU32;

use phonepad_core::geometry::Transform;

use super::*;

const HERE: Shot = Shot {
    crop: (100, 200),
    window: Point { x: 100, y: 200 },
};
const THERE: Shot = Shot {
    crop: (102, 200),
    window: Point { x: 102, y: 200 },
};
const QUIET: Pending = Pending {
    new_frame: false,
    keyframe: false,
    encoder_busy: false,
};
const FRAME: Pending = Pending {
    new_frame: true,
    keyframe: false,
    encoder_busy: false,
};
const KEY: Pending = Pending {
    new_frame: false,
    keyframe: true,
    encoder_busy: false,
};
const FRAME_WHILE_BUSY: Pending = Pending {
    new_frame: true,
    keyframe: false,
    encoder_busy: true,
};
const QUIET_WHILE_BUSY: Pending = Pending {
    new_frame: false,
    keyframe: false,
    encoder_busy: true,
};
const LONG_AGO: Option<Duration> = Some(Duration::from_secs(1));

fn size(w: u32, h: u32) -> Size {
    Size {
        w: NonZeroU32::new(w).unwrap(),
        h: NonZeroU32::new(h).unwrap(),
    }
}

fn monitor(name: &str, x: i32, w: u32, h: u32) -> Monitor {
    Monitor {
        name: name.to_owned(),
        origin: Point { x, y: 0 },
        size: size(w, h),
        pixels: size(w, h),
        transform: Transform::Normal,
    }
}

#[test]
fn no_monitor_has_no_target() {
    assert_eq!(
        target(Point { x: 0, y: 0 }, size(640, 480), &[], None),
        None
    );
}

fn window(output: &str, global: (i32, i32), local: (i32, i32)) -> Region {
    Region {
        output: output.to_owned(),
        global: Point {
            x: global.0,
            y: global.1,
        },
        local: Point {
            x: local.0,
            y: local.1,
        },
        size: size(960, 720),
    }
}

#[test]
fn target_on_one_monitor_crops_a_window_around_the_cursor() {
    assert_eq!(
        target(
            Point { x: 1000, y: 500 },
            size(640, 480),
            &[monitor("A", 0, 1920, 1080)],
            None
        ),
        Some(Target {
            shape: Shape {
                output: "A".to_owned(),
                source: Pixels {
                    width: 1920,
                    height: 1080
                },
                window: Pixels {
                    width: 960,
                    height: 720
                },
            },
            shot: Shot {
                crop: (520, 140),
                window: Point { x: 520, y: 140 },
            },
            window: window("A", (520, 140), (520, 140)),
        })
    );
}

#[test]
fn target_on_the_second_of_many_monitors_uses_its_source_and_local_crop() {
    assert_eq!(
        target(
            Point { x: 3000, y: 500 },
            size(640, 480),
            &[monitor("A", 0, 1920, 1080), monitor("B", 1920, 2560, 1440)],
            None
        ),
        Some(Target {
            shape: Shape {
                output: "B".to_owned(),
                source: Pixels {
                    width: 2560,
                    height: 1440
                },
                window: Pixels {
                    width: 960,
                    height: 720
                },
            },
            shot: Shot {
                crop: (600, 140),
                window: Point { x: 2520, y: 140 },
            },
            window: window("B", (2520, 140), (600, 140)),
        })
    );
}

#[test]
fn a_pointer_moving_inside_the_kept_window_keeps_the_shot() {
    let kept: Region = window("A", (520, 140), (520, 140));
    assert_eq!(
        target(
            Point { x: 1100, y: 520 },
            size(640, 480),
            &[monitor("A", 0, 1920, 1080)],
            Some(&kept)
        )
        .map(|t: Target| t.shot),
        Some(Shot {
            crop: (520, 140),
            window: Point { x: 520, y: 140 },
        })
    );
}

#[test]
fn no_request_is_no_keyframe() {
    assert!(!keyframe_due(false, None));
}

#[test]
fn a_request_before_any_keyframe_is_due() {
    assert!(keyframe_due(true, None));
}

#[test]
fn a_request_just_inside_the_interval_waits() {
    assert!(!keyframe_due(
        true,
        Some(MIN_KEYFRAME_INTERVAL - Duration::from_millis(1))
    ));
}

#[test]
fn a_request_one_interval_after_the_last_keyframe_is_due() {
    assert!(keyframe_due(true, Some(MIN_KEYFRAME_INTERVAL)));
}

#[test]
fn no_request_long_after_a_keyframe_is_no_keyframe() {
    assert!(!keyframe_due(false, Some(Duration::from_secs(10))));
}

const AR24: u32 = 0x3432_5241;
const XR24: u32 = 0x3432_5258;
const SURFACE: Buffer = Buffer {
    fourcc: AR24,
    modifier: 0,
    width: 2560,
    height: 1440,
};

fn offered(size: Option<(u32, u32)>, dmabuf: &[(u32, &[u64])]) -> Constraints {
    Constraints {
        size,
        dmabuf: dmabuf
            .iter()
            .map(|(fourcc, modifiers): &(u32, &[u64])| (*fourcc, modifiers.to_vec()))
            .collect(),
    }
}

#[test]
fn no_buffer_size_does_not_fit() {
    assert_eq!(
        fits(&offered(None, &[(AR24, &[0])]), SURFACE),
        Err(Misfit::NoSize)
    );
}

#[test]
fn the_offered_size_and_format_fit() {
    assert_eq!(
        fits(&offered(Some((2560, 1440)), &[(AR24, &[0])]), SURFACE),
        Ok(())
    );
}

#[test]
fn a_wider_buffer_does_not_fit() {
    assert_eq!(
        fits(&offered(Some((2561, 1440)), &[(AR24, &[0])]), SURFACE),
        Err(Misfit::Size {
            wanted: (2561, 1440),
            have: (2560, 1440)
        })
    );
}

#[test]
fn a_taller_buffer_does_not_fit() {
    assert!(matches!(
        fits(&offered(Some((2560, 1441)), &[(AR24, &[0])]), SURFACE),
        Err(Misfit::Size { .. })
    ));
}

#[test]
fn no_dmabuf_format_does_not_fit() {
    assert_eq!(
        fits(&offered(Some((2560, 1440)), &[]), SURFACE),
        Err(Misfit::Format {
            fourcc: AR24,
            modifier: 0,
            offered: Vec::new()
        })
    );
}

#[test]
fn the_format_without_the_linear_modifier_does_not_fit() {
    assert!(matches!(
        fits(&offered(Some((2560, 1440)), &[(AR24, &[1, 2])]), SURFACE),
        Err(Misfit::Format { .. })
    ));
}

#[test]
fn the_linear_modifier_on_another_format_does_not_fit() {
    assert!(matches!(
        fits(&offered(Some((2560, 1440)), &[(XR24, &[0])]), SURFACE),
        Err(Misfit::Format { .. })
    ));
}

#[test]
fn one_of_many_formats_fits() {
    assert_eq!(
        fits(
            &offered(Some((2560, 1440)), &[(XR24, &[0]), (AR24, &[7, 0])]),
            SURFACE
        ),
        Ok(())
    );
}

fn capturing() -> Slot<()> {
    Slot::Capturing {
        frame: (),
        session: 0,
    }
}

fn arrived(mut slots: Vec<Slot<()>>, index: usize, stamp: u64) -> (bool, Vec<Slot<()>>) {
    let newest: bool = arrive(&mut slots, index, stamp);
    (newest, slots)
}

#[test]
fn no_slot_held_has_no_frame() {
    assert_eq!(held(&[Slot::Free, capturing()]), None);
}

#[test]
fn the_held_slot_carries_its_stamp() {
    assert_eq!(
        held(&[capturing(), Slot::Held { stamp: 7 }, Slot::Free]),
        Some((1, 7))
    );
}

#[test]
fn the_first_frame_is_held() {
    assert_eq!(
        arrived(vec![capturing(), Slot::Free], 0, 5),
        (true, vec![Slot::Held { stamp: 5 }, Slot::Free])
    );
}

#[test]
fn a_later_frame_replaces_the_held_one() {
    assert_eq!(
        arrived(
            vec![Slot::Held { stamp: 5 }, capturing(), capturing()],
            1,
            6
        ),
        (true, vec![Slot::Free, Slot::Held { stamp: 6 }, capturing()])
    );
}

#[test]
fn a_frame_with_the_held_stamp_is_a_duplicate_and_freed() {
    assert_eq!(
        arrived(vec![Slot::Held { stamp: 5 }, capturing()], 1, 5),
        (false, vec![Slot::Held { stamp: 5 }, Slot::Free])
    );
}

#[test]
fn an_older_frame_is_stale_and_freed() {
    assert_eq!(
        arrived(vec![Slot::Held { stamp: 6 }, capturing()], 1, 5),
        (false, vec![Slot::Held { stamp: 6 }, Slot::Free])
    );
}

#[test]
fn no_free_slot_issues_nothing() {
    assert_eq!(
        to_issue(&[Slot::Held { stamp: 1 }, capturing()], &[false, false], 2),
        Vec::<usize>::new()
    );
}

#[test]
fn one_free_slot_below_the_limit_is_issued() {
    assert_eq!(
        to_issue(
            &[Slot::Held { stamp: 1 }, capturing(), Slot::Free],
            &[false, false, false],
            2
        ),
        vec![2]
    );
}

#[test]
fn many_free_slots_are_issued_up_to_the_limit() {
    assert_eq!(
        to_issue::<()>(
            &[Slot::Free, Slot::Free, Slot::Free],
            &[false, false, false],
            2
        ),
        vec![0, 1]
    );
}

#[test]
fn slots_already_capturing_count_against_the_limit() {
    assert_eq!(
        to_issue(
            &[capturing(), capturing(), Slot::Free],
            &[false, false, false],
            2
        ),
        Vec::<usize>::new()
    );
}

#[test]
fn a_free_slot_the_encoder_still_reads_is_not_issued() {
    assert_eq!(
        to_issue::<()>(&[Slot::Free, Slot::Free], &[true, false], 2),
        vec![1]
    );
}

#[test]
fn a_slot_with_no_pin_record_is_not_issued() {
    assert_eq!(
        to_issue::<()>(&[Slot::Free, Slot::Free], &[false], 2),
        vec![0]
    );
}

#[test]
fn stamp_joins_the_seconds_halves_and_adds_nanoseconds() {
    assert_eq!(stamp(1, 2, 3), ((1 << 32) + 2) * NANOS_PER_SECOND + 3);
}

#[test]
fn stamp_of_zero_is_zero() {
    assert_eq!(stamp(0, 0, 0), 0);
}

#[test]
fn nothing_new_is_not_encoded() {
    assert_eq!(schedule(QUIET, Some(HERE), HERE, LONG_AGO), Schedule::Idle);
}

#[test]
fn nothing_shown_yet_is_encoded_at_once() {
    assert_eq!(schedule(QUIET, None, HERE, None), Schedule::EncodeNow);
}

#[test]
fn a_new_frame_is_encoded() {
    assert_eq!(
        schedule(FRAME, Some(HERE), HERE, LONG_AGO),
        Schedule::EncodeNow
    );
}

#[test]
fn a_keyframe_request_is_encoded() {
    assert_eq!(
        schedule(KEY, Some(HERE), HERE, LONG_AGO),
        Schedule::EncodeNow
    );
}

#[test]
fn a_moved_pointer_is_encoded() {
    assert_eq!(
        schedule(QUIET, Some(HERE), THERE, LONG_AGO),
        Schedule::EncodeNow
    );
}

#[test]
fn a_change_while_the_encoder_is_full_waits_for_it() {
    assert_eq!(
        schedule(FRAME_WHILE_BUSY, Some(HERE), HERE, LONG_AGO),
        Schedule::Busy
    );
}

#[test]
fn no_change_while_the_encoder_is_full_is_idle() {
    assert_eq!(
        schedule(QUIET_WHILE_BUSY, Some(HERE), HERE, LONG_AGO),
        Schedule::Idle
    );
}

#[test]
fn a_change_too_soon_waits_for_the_rest_of_the_interval() {
    assert_eq!(
        schedule(FRAME, Some(HERE), HERE, Some(Duration::from_millis(10))),
        Schedule::WaitFor(MIN_ENCODE_INTERVAL - Duration::from_millis(10))
    );
}

#[test]
fn a_change_exactly_one_interval_later_is_encoded() {
    assert_eq!(
        schedule(FRAME, Some(HERE), HERE, Some(MIN_ENCODE_INTERVAL)),
        Schedule::EncodeNow
    );
}

#[test]
fn a_change_just_before_the_interval_waits() {
    assert_eq!(
        schedule(
            FRAME,
            Some(HERE),
            HERE,
            Some(MIN_ENCODE_INTERVAL - Duration::from_micros(1))
        ),
        Schedule::WaitFor(Duration::from_micros(1))
    );
}
