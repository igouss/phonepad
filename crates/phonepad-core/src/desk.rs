use crate::decision::Decision;

pub const PHONE_QUIET_MS: u64 = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerSize {
    Normal,
    Big,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pointer {
    Normal,
    Big { last_phone_input_ms: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Desk {
    pub viewers: u32,
    pub pointer: Pointer,
}

impl Desk {
    pub fn initial() -> Desk {
        Desk {
            viewers: 0,
            pointer: Pointer::Normal,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeskInput {
    ViewerJoined,
    ViewerLeft,
    PhoneInput { at_ms: u64 },
    InputActivity { at_ms: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeskEvent {
    ViewerJoined,
    ViewerLeft,
    PointerEnlarged { at_ms: u64 },
    PhoneActive { at_ms: u64 },
    PointerRestored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeskCommand {
    StartCapture,
    StopCapture,
    SetPointer(PointerSize),
}

pub fn decide(state: &Desk, input: DeskInput) -> Decision<DeskEvent, DeskCommand> {
    match input {
        DeskInput::ViewerJoined => viewer_joined(state),
        DeskInput::ViewerLeft => viewer_left(state),
        DeskInput::PhoneInput { at_ms } => phone_input(state, at_ms),
        DeskInput::InputActivity { at_ms } => input_activity(state, at_ms),
    }
}

fn viewer_joined(state: &Desk) -> Decision<DeskEvent, DeskCommand> {
    let commands: Vec<DeskCommand> = if state.viewers == 0 {
        vec![DeskCommand::StartCapture]
    } else {
        Vec::new()
    };
    Decision {
        events: vec![DeskEvent::ViewerJoined],
        commands,
    }
}

fn viewer_left(state: &Desk) -> Decision<DeskEvent, DeskCommand> {
    match (state.viewers, state.pointer) {
        (0, Pointer::Normal | Pointer::Big { .. }) => Decision::none(),
        (1, Pointer::Normal) => Decision {
            events: vec![DeskEvent::ViewerLeft],
            commands: vec![DeskCommand::StopCapture],
        },
        (1, Pointer::Big { .. }) => Decision {
            events: vec![DeskEvent::ViewerLeft, DeskEvent::PointerRestored],
            commands: vec![
                DeskCommand::StopCapture,
                DeskCommand::SetPointer(PointerSize::Normal),
            ],
        },
        (2.., Pointer::Normal | Pointer::Big { .. }) => Decision {
            events: vec![DeskEvent::ViewerLeft],
            commands: Vec::new(),
        },
    }
}

fn phone_input(state: &Desk, at_ms: u64) -> Decision<DeskEvent, DeskCommand> {
    match state.pointer {
        Pointer::Normal => Decision {
            events: vec![DeskEvent::PointerEnlarged { at_ms }],
            commands: vec![DeskCommand::SetPointer(PointerSize::Big)],
        },
        Pointer::Big { .. } => Decision {
            events: vec![DeskEvent::PhoneActive { at_ms }],
            commands: Vec::new(),
        },
    }
}

fn input_activity(state: &Desk, at_ms: u64) -> Decision<DeskEvent, DeskCommand> {
    match state.pointer {
        Pointer::Big {
            last_phone_input_ms,
        } if at_ms.saturating_sub(last_phone_input_ms) > PHONE_QUIET_MS => Decision {
            events: vec![DeskEvent::PointerRestored],
            commands: vec![DeskCommand::SetPointer(PointerSize::Normal)],
        },
        Pointer::Normal | Pointer::Big { .. } => Decision::none(),
    }
}

pub fn evolve(state: Desk, event: &DeskEvent) -> Desk {
    let pointer: Pointer = match (state.pointer, *event) {
        (
            Pointer::Normal | Pointer::Big { .. },
            DeskEvent::PointerEnlarged { at_ms } | DeskEvent::PhoneActive { at_ms },
        ) => Pointer::Big {
            last_phone_input_ms: at_ms,
        },
        (Pointer::Normal | Pointer::Big { .. }, DeskEvent::PointerRestored) => Pointer::Normal,
        (pointer, DeskEvent::ViewerJoined | DeskEvent::ViewerLeft) => pointer,
    };
    let viewers: u32 = match event {
        DeskEvent::ViewerJoined => state.viewers.saturating_add(1),
        DeskEvent::ViewerLeft => state.viewers.saturating_sub(1),
        DeskEvent::PointerEnlarged { .. }
        | DeskEvent::PhoneActive { .. }
        | DeskEvent::PointerRestored => state.viewers,
    };
    Desk { viewers, pointer }
}

pub fn all_states() -> Vec<Desk> {
    let pointers: [Pointer; 2] = [
        Pointer::Normal,
        Pointer::Big {
            last_phone_input_ms: 1000,
        },
    ];
    (0..=2)
        .flat_map(|viewers: u32| pointers.map(|pointer: Pointer| Desk { viewers, pointer }))
        .collect()
}

pub fn all_inputs() -> Vec<DeskInput> {
    vec![
        DeskInput::ViewerJoined,
        DeskInput::ViewerLeft,
        DeskInput::PhoneInput { at_ms: 1100 },
        DeskInput::InputActivity { at_ms: 1100 },
        DeskInput::InputActivity { at_ms: 1400 },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn variant_index(pointer: &Pointer) -> usize {
        match pointer {
            Pointer::Normal => 0,
            Pointer::Big { .. } => 1,
        }
    }

    fn input_index(input: &DeskInput) -> usize {
        match input {
            DeskInput::ViewerJoined => 0,
            DeskInput::ViewerLeft => 1,
            DeskInput::PhoneInput { .. } => 2,
            DeskInput::InputActivity { .. } => 3,
        }
    }

    fn big(last_phone_input_ms: u64) -> Pointer {
        Pointer::Big {
            last_phone_input_ms,
        }
    }

    fn desk(viewers: u32, pointer: Pointer) -> Desk {
        Desk { viewers, pointer }
    }

    fn decision(
        events: Vec<DeskEvent>,
        commands: Vec<DeskCommand>,
    ) -> Decision<DeskEvent, DeskCommand> {
        Decision { events, commands }
    }

    fn restore() -> Decision<DeskEvent, DeskCommand> {
        decision(
            vec![DeskEvent::PointerRestored],
            vec![DeskCommand::SetPointer(PointerSize::Normal)],
        )
    }

    fn activity(at_ms: u64) -> DeskInput {
        DeskInput::InputActivity { at_ms }
    }

    #[test]
    fn all_states_cover_every_pointer_variant() {
        assert_eq!(
            all_states()
                .iter()
                .map(|d: &Desk| variant_index(&d.pointer))
                .collect::<BTreeSet<usize>>(),
            BTreeSet::from([0, 1])
        );
    }

    #[test]
    fn all_inputs_cover_every_variant() {
        assert_eq!(
            all_inputs()
                .iter()
                .map(input_index)
                .collect::<BTreeSet<usize>>(),
            BTreeSet::from([0, 1, 2, 3])
        );
    }

    #[test]
    fn all_states_lists_viewers_by_pointer() {
        assert_eq!(
            all_states(),
            vec![
                desk(0, Pointer::Normal),
                desk(0, big(1000)),
                desk(1, Pointer::Normal),
                desk(1, big(1000)),
                desk(2, Pointer::Normal),
                desk(2, big(1000)),
            ]
        );
    }

    #[test]
    fn all_inputs_lists_every_input() {
        assert_eq!(
            all_inputs(),
            vec![
                DeskInput::ViewerJoined,
                DeskInput::ViewerLeft,
                DeskInput::PhoneInput { at_ms: 1100 },
                activity(1100),
                activity(1400),
            ]
        );
    }

    #[test]
    fn initial_desk_is_empty_with_normal_pointer() {
        assert_eq!(Desk::initial(), desk(0, Pointer::Normal));
    }

    #[test]
    fn first_viewer_starts_capture() {
        assert_eq!(
            decide(&desk(0, Pointer::Normal), DeskInput::ViewerJoined),
            decision(
                vec![DeskEvent::ViewerJoined],
                vec![DeskCommand::StartCapture]
            )
        );
    }

    #[test]
    fn second_viewer_only_joins() {
        assert_eq!(
            decide(&desk(1, Pointer::Normal), DeskInput::ViewerJoined),
            decision(vec![DeskEvent::ViewerJoined], vec![])
        );
    }

    #[test]
    fn viewer_left_with_no_viewers_does_nothing() {
        assert_eq!(
            decide(&desk(0, big(1000)), DeskInput::ViewerLeft),
            Decision::none()
        );
    }

    #[test]
    fn last_viewer_left_stops_capture() {
        assert_eq!(
            decide(&desk(1, Pointer::Normal), DeskInput::ViewerLeft),
            decision(vec![DeskEvent::ViewerLeft], vec![DeskCommand::StopCapture])
        );
    }

    #[test]
    fn last_viewer_left_with_big_pointer_restores_it() {
        assert_eq!(
            decide(&desk(1, big(1000)), DeskInput::ViewerLeft),
            decision(
                vec![DeskEvent::ViewerLeft, DeskEvent::PointerRestored],
                vec![
                    DeskCommand::StopCapture,
                    DeskCommand::SetPointer(PointerSize::Normal),
                ]
            )
        );
    }

    #[test]
    fn one_of_many_viewers_left_keeps_big_pointer() {
        assert_eq!(
            decide(&desk(2, big(1000)), DeskInput::ViewerLeft),
            decision(vec![DeskEvent::ViewerLeft], vec![])
        );
    }

    #[test]
    fn phone_input_enlarges_normal_pointer() {
        assert_eq!(
            decide(
                &desk(1, Pointer::Normal),
                DeskInput::PhoneInput { at_ms: 1100 }
            ),
            decision(
                vec![DeskEvent::PointerEnlarged { at_ms: 1100 }],
                vec![DeskCommand::SetPointer(PointerSize::Big)]
            )
        );
    }

    #[test]
    fn phone_input_with_big_pointer_marks_activity() {
        assert_eq!(
            decide(&desk(1, big(1000)), DeskInput::PhoneInput { at_ms: 1100 }),
            decision(vec![DeskEvent::PhoneActive { at_ms: 1100 }], vec![])
        );
    }

    #[test]
    fn activity_with_normal_pointer_does_nothing() {
        assert_eq!(
            decide(&desk(1, Pointer::Normal), activity(5000)),
            Decision::none()
        );
    }

    #[test]
    fn activity_soon_after_phone_input_is_the_phones_own() {
        assert_eq!(
            decide(&desk(1, big(1000)), activity(1100)),
            Decision::none()
        );
    }

    #[test]
    fn activity_exactly_at_quiet_limit_is_still_the_phones() {
        assert_eq!(
            decide(&desk(1, big(1000)), activity(1000 + PHONE_QUIET_MS)),
            Decision::none()
        );
    }

    #[test]
    fn activity_just_after_quiet_limit_is_real_input_and_restores_pointer() {
        assert_eq!(
            decide(&desk(1, big(1000)), activity(1001 + PHONE_QUIET_MS)),
            restore()
        );
    }

    #[test]
    fn activity_stamped_before_phone_input_is_the_phones() {
        assert_eq!(decide(&desk(1, big(1000)), activity(0)), Decision::none());
    }

    #[test]
    fn viewer_joined_adds_viewer() {
        assert_eq!(
            evolve(desk(1, big(1000)), &DeskEvent::ViewerJoined),
            desk(2, big(1000))
        );
    }

    #[test]
    fn viewer_joined_saturates() {
        assert_eq!(
            evolve(desk(u32::MAX, Pointer::Normal), &DeskEvent::ViewerJoined),
            desk(u32::MAX, Pointer::Normal)
        );
    }

    #[test]
    fn viewer_left_removes_viewer() {
        assert_eq!(
            evolve(desk(2, big(1000)), &DeskEvent::ViewerLeft),
            desk(1, big(1000))
        );
    }

    #[test]
    fn viewer_left_saturates_at_zero() {
        assert_eq!(
            evolve(desk(0, Pointer::Normal), &DeskEvent::ViewerLeft),
            desk(0, Pointer::Normal)
        );
    }

    #[test]
    fn pointer_enlarged_makes_big_pointer() {
        assert_eq!(
            evolve(
                desk(1, Pointer::Normal),
                &DeskEvent::PointerEnlarged { at_ms: 1000 }
            ),
            desk(1, big(1000))
        );
    }

    #[test]
    fn phone_active_updates_input_time() {
        assert_eq!(
            evolve(desk(1, big(1000)), &DeskEvent::PhoneActive { at_ms: 2000 }),
            desk(1, big(2000))
        );
    }

    #[test]
    fn pointer_restored_makes_pointer_normal() {
        assert_eq!(
            evolve(desk(2, big(1000)), &DeskEvent::PointerRestored),
            desk(2, Pointer::Normal)
        );
    }
}
