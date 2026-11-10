use super::*;

fn requested() -> Slot<()> {
    requested_with(Vec::new())
}

fn requested_with(damage: Vec<Crop>) -> Slot<()> {
    Slot::Requested {
        frame: (),
        session: 0,
        damage,
    }
}

fn copying(stamp: u64) -> Slot<()> {
    copying_with(Vec::new(), stamp)
}

fn copying_with(damage: Vec<Crop>, stamp: u64) -> Slot<()> {
    Slot::Copying {
        frame: (),
        session: 0,
        damage,
        stamp,
    }
}

fn copying_on(session: usize) -> Slot<()> {
    Slot::Copying {
        frame: (),
        session,
        damage: Vec::new(),
        stamp: 1,
    }
}

fn arrived(mut slots: Vec<Slot<()>>, index: usize) -> Vec<Slot<()>> {
    arrive(&mut slots, index);
    slots
}

fn after_presentation(mut slots: Vec<Slot<()>>, index: usize, at: u64) -> Vec<Slot<()>> {
    presented(&mut slots, index, at);
    slots
}

#[test]
fn no_slot_held_has_no_frame() {
    assert_eq!(held(&[Slot::Free, requested(), copying(3)]), None);
}

#[test]
fn the_held_slot_carries_its_stamp() {
    assert_eq!(
        held(&[requested(), Slot::Held { stamp: 7 }, Slot::Free]),
        Some((1, 7))
    );
}

#[test]
fn the_first_frame_is_held_at_its_presentation_stamp() {
    assert_eq!(
        arrived(vec![copying(5), Slot::Free], 0),
        vec![Slot::Held { stamp: 5 }, Slot::Free]
    );
}

#[test]
fn a_later_frame_replaces_the_held_one() {
    assert_eq!(
        arrived(vec![Slot::Held { stamp: 5 }, copying(6), requested()], 1),
        vec![Slot::Free, Slot::Held { stamp: 6 }, requested()]
    );
}

#[test]
fn a_frame_with_the_held_stamp_is_a_duplicate_and_freed() {
    assert_eq!(
        arrived(vec![Slot::Held { stamp: 5 }, copying(5)], 1),
        vec![Slot::Held { stamp: 5 }, Slot::Free]
    );
}

#[test]
fn an_older_frame_is_stale_and_freed() {
    assert_eq!(
        arrived(vec![Slot::Held { stamp: 6 }, copying(5)], 1),
        vec![Slot::Held { stamp: 6 }, Slot::Free]
    );
}

#[test]
fn a_frame_ready_without_presentation_time_is_held_at_stamp_zero_when_nothing_is_held() {
    assert_eq!(arrived(vec![requested()], 0), vec![Slot::Held { stamp: 0 }]);
}

#[test]
fn a_frame_ready_without_presentation_time_never_replaces_a_held_one() {
    assert_eq!(
        arrived(vec![Slot::Held { stamp: 0 }, requested()], 1),
        vec![Slot::Held { stamp: 0 }, Slot::Free]
    );
}

#[test]
fn a_ready_on_a_free_slot_changes_nothing() {
    assert_eq!(
        arrived(vec![Slot::Held { stamp: 4 }, Slot::Free], 1),
        vec![Slot::Held { stamp: 4 }, Slot::Free]
    );
}

#[test]
fn a_ready_on_the_held_slot_changes_nothing() {
    assert_eq!(
        arrived(vec![Slot::Held { stamp: 4 }], 0),
        vec![Slot::Held { stamp: 4 }]
    );
}

#[test]
fn a_ready_on_no_slot_changes_nothing() {
    assert_eq!(arrived(vec![copying(4)], 1), vec![copying(4)]);
}

#[test]
fn presentation_time_starts_the_copy_and_keeps_the_damage() {
    assert_eq!(
        after_presentation(vec![Slot::Free, requested_with(vec![INSIDE])], 1, 9),
        vec![Slot::Free, copying_with(vec![INSIDE], 9)]
    );
}

#[test]
fn a_second_presentation_time_restamps_the_copy() {
    assert_eq!(
        after_presentation(vec![copying_with(vec![OUTSIDE], 9)], 0, 11),
        vec![copying_with(vec![OUTSIDE], 11)]
    );
}

#[test]
fn presentation_time_on_a_free_slot_changes_nothing() {
    assert_eq!(after_presentation(vec![Slot::Free], 0, 9), vec![Slot::Free]);
}

#[test]
fn presentation_time_on_a_held_slot_changes_nothing() {
    assert_eq!(
        after_presentation(vec![Slot::Held { stamp: 3 }], 0, 9),
        vec![Slot::Held { stamp: 3 }]
    );
}

#[test]
fn presentation_time_on_no_slot_changes_nothing() {
    assert_eq!(
        after_presentation(vec![requested()], 1, 9),
        vec![requested()]
    );
}

#[test]
fn no_free_slot_issues_nothing() {
    assert_eq!(
        to_issue(
            &[Slot::Held { stamp: 1 }, copying_on(0)],
            &[false, false],
            &[true, true]
        ),
        None
    );
}

#[test]
fn no_session_issues_nothing() {
    assert_eq!(to_issue::<()>(&[Slot::Free], &[false], &[]), None);
}

#[test]
fn a_session_not_ready_issues_nothing() {
    assert_eq!(to_issue::<()>(&[Slot::Free], &[false], &[false]), None);
}

#[test]
fn one_free_slot_is_issued_on_the_idle_session() {
    assert_eq!(
        to_issue(
            &[Slot::Held { stamp: 1 }, copying_on(0), Slot::Free],
            &[false, false, false],
            &[true, true]
        ),
        Some((2, 1))
    );
}

#[test]
fn many_free_slots_and_sessions_issue_only_the_first_of_each() {
    assert_eq!(
        to_issue::<()>(
            &[Slot::Free, Slot::Free, Slot::Free],
            &[false, false, false],
            &[true, true]
        ),
        Some((0, 0))
    );
}

#[test]
fn a_frame_waiting_for_its_commit_blocks_every_other_request() {
    assert_eq!(
        to_issue(
            &[requested(), Slot::Free, Slot::Free],
            &[false, false, false],
            &[true, true]
        ),
        None
    );
}

#[test]
fn every_session_copying_issues_nothing() {
    assert_eq!(
        to_issue(
            &[copying_on(1), copying_on(0), Slot::Free],
            &[false, false, false],
            &[true, true]
        ),
        None
    );
}

#[test]
fn a_free_slot_the_encoder_still_reads_is_not_issued() {
    assert_eq!(
        to_issue::<()>(&[Slot::Free, Slot::Free], &[true, false], &[true]),
        Some((1, 0))
    );
}

#[test]
fn a_slot_with_no_pin_record_is_not_issued() {
    assert_eq!(
        to_issue::<()>(&[Slot::Free, Slot::Free], &[true], &[true]),
        None
    );
}

const OUTSIDE: Crop = Crop {
    x: 0,
    y: 0,
    w: NonZeroU32::new(50).unwrap(),
    h: NonZeroU32::new(50).unwrap(),
};
const INSIDE: Crop = Crop {
    x: 300,
    y: 300,
    w: NonZeroU32::new(50).unwrap(),
    h: NonZeroU32::new(50).unwrap(),
};
const SHOWN: Shown = Shown {
    shot: HERE,
    stamp: 5,
};

fn after_damage(mut slots: Vec<Slot<()>>, index: usize, rect: Crop) -> Vec<Slot<()>> {
    damaged(&mut slots, index, rect);
    slots
}

#[test]
fn damage_to_a_requested_slot_is_collected() {
    assert_eq!(
        after_damage(vec![Slot::Free, requested()], 1, INSIDE),
        vec![Slot::Free, requested_with(vec![INSIDE])]
    );
}

#[test]
fn damage_to_a_copying_slot_is_collected() {
    assert_eq!(
        after_damage(vec![copying(2)], 0, INSIDE),
        vec![copying_with(vec![INSIDE], 2)]
    );
}

#[test]
fn more_damage_to_a_slot_is_added_to_what_it_has() {
    assert_eq!(
        after_damage(vec![requested_with(vec![INSIDE])], 0, OUTSIDE),
        vec![requested_with(vec![INSIDE, OUTSIDE])]
    );
}

#[test]
fn damage_to_a_free_slot_is_dropped() {
    assert_eq!(
        after_damage(vec![Slot::Free, requested()], 0, INSIDE),
        vec![Slot::Free, requested()]
    );
}

#[test]
fn damage_to_a_held_slot_is_dropped() {
    assert_eq!(
        after_damage(vec![Slot::Held { stamp: 1 }], 0, INSIDE),
        vec![Slot::Held { stamp: 1 }]
    );
}

#[test]
fn damage_to_no_slot_is_dropped() {
    assert_eq!(
        after_damage(vec![requested()], 1, INSIDE),
        vec![requested()]
    );
}

#[test]
fn a_frame_when_nothing_is_shown_changes_it() {
    assert!(changes_shown(&copying(1), None));
}

#[test]
fn an_untimed_frame_when_nothing_is_shown_changes_it() {
    assert!(changes_shown(&requested(), None));
}

#[test]
fn an_untimed_frame_damaged_inside_changes_nothing_shown() {
    assert!(!changes_shown(&requested_with(vec![INSIDE]), Some(SHOWN)));
}

#[test]
fn a_frame_with_no_damage_changes_nothing() {
    assert!(!changes_shown(&copying(6), Some(SHOWN)));
}

#[test]
fn a_newer_frame_damaged_inside_the_shown_crop_changes_it() {
    assert!(changes_shown(&copying_with(vec![INSIDE], 6), Some(SHOWN)));
}

#[test]
fn a_newer_frame_damaged_only_outside_the_shown_crop_changes_nothing() {
    assert!(!changes_shown(&copying_with(vec![OUTSIDE], 6), Some(SHOWN)));
}

#[test]
fn a_newer_frame_damaged_in_many_places_one_inside_changes_it() {
    assert!(changes_shown(
        &copying_with(vec![OUTSIDE, INSIDE], 6),
        Some(SHOWN)
    ));
}

#[test]
fn a_newer_frame_damaged_in_many_places_all_outside_changes_nothing() {
    assert!(!changes_shown(
        &copying_with(vec![OUTSIDE, crop(2000, 2000, 10, 10)], 6),
        Some(SHOWN)
    ));
}

#[test]
fn a_frame_of_the_shown_moment_changes_nothing() {
    assert!(!changes_shown(&copying_with(vec![INSIDE], 5), Some(SHOWN)));
}

#[test]
fn an_older_frame_damaged_inside_changes_nothing() {
    assert!(!changes_shown(&copying_with(vec![INSIDE], 4), Some(SHOWN)));
}

#[test]
fn a_free_slot_changes_nothing() {
    assert!(!changes_shown(&Slot::<()>::Free, None));
}

#[test]
fn a_held_slot_changes_nothing() {
    assert!(!changes_shown(&Slot::<()>::Held { stamp: 6 }, None));
}
