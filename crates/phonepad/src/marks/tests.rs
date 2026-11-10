use super::*;

const PHONE: PhoneId = PhoneId(1);
const OTHER: PhoneId = PhoneId(2);

fn entries(applied: &Applied) -> Vec<(PhoneId, u32)> {
    applied
        .iter()
        .map(|(phone, seq): (&PhoneId, &u32)| (*phone, *seq))
        .collect()
}

fn read_now(marks: &Marks) -> Option<Vec<(PhoneId, u32)>> {
    marks
        .read(|| ())
        .map(|(applied, ()): (Applied, ())| entries(&applied))
}

#[test]
fn each_phone_gets_its_own_id() {
    let ids: PhoneIds = PhoneIds::default();
    assert_eq!([ids.next(), ids.next()], [PhoneId(0), PhoneId(1)]);
}

#[test]
fn a_phone_that_never_moved_has_applied_nothing() {
    assert_eq!(applied_seq(&Applied::new(), PHONE), 0);
}

#[test]
fn a_phone_has_applied_its_own_last_move_not_another_phones() {
    let applied: Applied = [(PHONE, 4), (OTHER, 9)].into_iter().collect();
    assert_eq!(applied_seq(&applied, PHONE), 4);
}

#[test]
fn no_moves_read_as_none_applied() {
    assert_eq!(read_now(&Marks::default()), Some(Vec::new()));
}

#[test]
fn a_move_is_read_once_it_is_emitted() {
    let marks: Marks = Marks::default();
    marks.moved(PHONE, 1, || ());
    assert_eq!(read_now(&marks), Some(vec![(PHONE, 1)]));
}

#[test]
fn many_moves_keep_each_phones_last() {
    let marks: Marks = Marks::default();
    marks.moved(PHONE, 1, || ());
    marks.moved(OTHER, 7, || ());
    marks.moved(PHONE, 2, || ());
    assert_eq!(read_now(&marks), Some(vec![(PHONE, 2), (OTHER, 7)]));
}

#[test]
fn a_move_hands_back_what_its_emit_returned() {
    assert_eq!(Marks::default().moved(PHONE, 1, || 42), 42);
}

#[test]
fn a_read_while_a_move_is_being_emitted_is_refused() {
    let marks: Marks = Marks::default();
    assert_eq!(marks.moved(PHONE, 1, || read_now(&marks)), None);
}

#[test]
fn a_query_that_a_move_overlaps_is_refused() {
    let marks: Marks = Marks::default();
    assert_eq!(
        marks
            .read(|| marks.moved(PHONE, 1, || ()))
            .map(|(applied, ()): (Applied, ())| entries(&applied)),
        None
    );
}

#[test]
fn a_query_answers_with_the_marks_before_it() {
    let marks: Marks = Marks::default();
    marks.moved(PHONE, 3, || ());
    assert_eq!(
        marks
            .read(|| "here")
            .map(|(applied, at): (Applied, &str)| (entries(&applied), at)),
        Some((vec![(PHONE, 3)], "here"))
    );
}

#[test]
fn a_forgotten_phone_has_no_mark_and_the_others_keep_theirs() {
    let marks: Marks = Marks::default();
    marks.moved(PHONE, 3, || ());
    marks.moved(OTHER, 5, || ());
    marks.forget(PHONE);
    assert_eq!(read_now(&marks), Some(vec![(OTHER, 5)]));
}
