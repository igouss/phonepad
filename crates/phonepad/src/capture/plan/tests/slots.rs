use super::*;

fn capturing() -> Slot<()> {
    Slot::Capturing {
        frame: (),
        session: 0,
        damage: Vec::new(),
    }
}

fn capturing_with(damage: Vec<Crop>) -> Slot<()> {
    Slot::Capturing {
        frame: (),
        session: 0,
        damage,
    }
}

fn arrived(mut slots: Vec<Slot<()>>, index: usize, stamp: u64) -> Vec<Slot<()>> {
    arrive(&mut slots, index, stamp);
    slots
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
        vec![Slot::Held { stamp: 5 }, Slot::Free]
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
        vec![Slot::Free, Slot::Held { stamp: 6 }, capturing()]
    );
}

#[test]
fn a_frame_with_the_held_stamp_is_a_duplicate_and_freed() {
    assert_eq!(
        arrived(vec![Slot::Held { stamp: 5 }, capturing()], 1, 5),
        vec![Slot::Held { stamp: 5 }, Slot::Free]
    );
}

#[test]
fn an_older_frame_is_stale_and_freed() {
    assert_eq!(
        arrived(vec![Slot::Held { stamp: 6 }, capturing()], 1, 5),
        vec![Slot::Held { stamp: 6 }, Slot::Free]
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
fn damage_to_a_capturing_slot_is_collected() {
    assert_eq!(
        after_damage(vec![Slot::Free, capturing()], 1, INSIDE),
        vec![Slot::Free, capturing_with(vec![INSIDE])]
    );
}

#[test]
fn more_damage_to_a_capturing_slot_is_added_to_what_it_has() {
    assert_eq!(
        after_damage(vec![capturing_with(vec![INSIDE])], 0, OUTSIDE),
        vec![capturing_with(vec![INSIDE, OUTSIDE])]
    );
}

#[test]
fn damage_to_a_free_slot_is_dropped() {
    assert_eq!(
        after_damage(vec![Slot::Free, capturing()], 0, INSIDE),
        vec![Slot::Free, capturing()]
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
        after_damage(vec![capturing()], 1, INSIDE),
        vec![capturing()]
    );
}

#[test]
fn a_frame_when_nothing_is_shown_changes_it() {
    assert!(changes_shown(&capturing(), 1, None));
}

#[test]
fn a_frame_with_no_damage_changes_nothing() {
    assert!(!changes_shown(&capturing(), 6, Some(SHOWN)));
}

#[test]
fn a_newer_frame_damaged_inside_the_shown_crop_changes_it() {
    assert!(changes_shown(&capturing_with(vec![INSIDE]), 6, Some(SHOWN)));
}

#[test]
fn a_newer_frame_damaged_only_outside_the_shown_crop_changes_nothing() {
    assert!(!changes_shown(
        &capturing_with(vec![OUTSIDE]),
        6,
        Some(SHOWN)
    ));
}

#[test]
fn a_newer_frame_damaged_in_many_places_one_inside_changes_it() {
    assert!(changes_shown(
        &capturing_with(vec![OUTSIDE, INSIDE]),
        6,
        Some(SHOWN)
    ));
}

#[test]
fn a_newer_frame_damaged_in_many_places_all_outside_changes_nothing() {
    assert!(!changes_shown(
        &capturing_with(vec![OUTSIDE, crop(2000, 2000, 10, 10)]),
        6,
        Some(SHOWN)
    ));
}

#[test]
fn a_frame_of_the_shown_moment_from_the_other_session_changes_nothing() {
    assert!(!changes_shown(
        &capturing_with(vec![INSIDE]),
        5,
        Some(SHOWN)
    ));
}

#[test]
fn an_older_frame_damaged_inside_changes_nothing() {
    assert!(!changes_shown(
        &capturing_with(vec![INSIDE]),
        4,
        Some(SHOWN)
    ));
}

#[test]
fn a_free_slot_changes_nothing() {
    assert!(!changes_shown(&Slot::<()>::Free, 6, None));
}

#[test]
fn a_held_slot_changes_nothing() {
    assert!(!changes_shown(&Slot::<()>::Held { stamp: 6 }, 6, None));
}
