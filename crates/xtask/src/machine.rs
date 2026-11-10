use crate::mermaid::Ids;
use phonepad_core::decision::Decision;
use std::fmt::Debug;

#[derive(Debug, PartialEq, Eq)]
pub struct State {
    pub id: String,
    pub label: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Transition {
    pub from: String,
    pub to: String,
    pub input: String,
    pub commands: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Machine {
    pub states: Vec<State>,
    pub transitions: Vec<Transition>,
}

struct Registry {
    ids: Ids,
    states: Vec<State>,
}

pub struct Labels<S, I> {
    pub state: fn(&S) -> String,
    pub input: fn(&I) -> String,
}

pub fn debug<T: Debug>(value: &T) -> String {
    format!("{value:?}")
}

impl<S: Debug, I: Debug> Labels<S, I> {
    pub fn debug() -> Labels<S, I> {
        Labels {
            state: debug::<S>,
            input: debug::<I>,
        }
    }
}

impl Registry {
    fn id(&mut self, label: String) -> String {
        let id: String = self.ids.id("s_", &label);
        if !self.states.iter().any(|s| s.id == id) {
            self.states.push(State {
                id: id.clone(),
                label,
            });
        }
        id
    }
}

pub fn explore<S, I, E, C>(
    states: Vec<S>,
    inputs: Vec<I>,
    decide: fn(&S, I) -> Decision<E, C>,
    evolve: fn(S, &E) -> S,
    labels: Labels<S, I>,
) -> Machine
where
    S: Clone,
    I: Clone,
    C: Debug,
{
    let mut registry: Registry = Registry {
        ids: Ids::default(),
        states: Vec::new(),
    };
    for state in &states {
        registry.id((labels.state)(state));
    }
    let mut transitions: Vec<Transition> = Vec::new();
    for state in &states {
        for input in &inputs {
            let decision: Decision<E, C> = decide(state, input.clone());
            if decision.events.is_empty() {
                continue;
            }
            let target: S = decision.events.iter().fold(state.clone(), evolve);
            transitions.push(Transition {
                from: registry.id((labels.state)(state)),
                to: registry.id((labels.state)(&target)),
                input: (labels.input)(input),
                commands: decision.commands.iter().map(|c| format!("{c:?}")).collect(),
            });
        }
    }
    Machine {
        states: registry.states,
        transitions,
    }
}

#[cfg(test)]
mod tests {
    use super::{Labels, Machine, State, Transition, explore};
    use phonepad_core::decision::Decision;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Lamp {
        Off,
        On,
    }

    #[derive(Clone, Copy, Debug)]
    enum Press {
        Toggle,
        Ignore,
    }

    #[derive(Debug)]
    enum Flipped {
        Once,
    }

    #[derive(Debug)]
    enum Wire {
        PowerOn,
        Log,
    }

    fn decide(state: &Lamp, input: Press) -> Decision<Flipped, Wire> {
        match (state, input) {
            (Lamp::Off, Press::Toggle) => Decision {
                events: vec![Flipped::Once],
                commands: vec![Wire::PowerOn, Wire::Log],
            },
            (Lamp::On, Press::Toggle) => Decision {
                events: vec![Flipped::Once, Flipped::Once],
                commands: vec![],
            },
            (Lamp::Off | Lamp::On, Press::Ignore) => Decision::none(),
        }
    }

    fn evolve(state: Lamp, event: &Flipped) -> Lamp {
        match (state, event) {
            (Lamp::Off, Flipped::Once) => Lamp::On,
            (Lamp::On, Flipped::Once) => Lamp::Off,
        }
    }

    fn state(id: &str, label: &str) -> State {
        State {
            id: id.to_string(),
            label: label.to_string(),
        }
    }

    #[test]
    fn no_states_give_an_empty_machine() {
        assert_eq!(
            explore(vec![], vec![Press::Toggle], decide, evolve, Labels::debug()),
            Machine {
                states: vec![],
                transitions: vec![]
            }
        );
    }

    #[test]
    fn inputs_without_events_produce_no_transitions() {
        let machine: Machine = explore(
            vec![Lamp::Off],
            vec![Press::Ignore],
            decide,
            evolve,
            Labels::debug(),
        );
        assert_eq!(
            machine,
            Machine {
                states: vec![state("s_Off", "Off")],
                transitions: vec![]
            }
        );
    }

    #[test]
    fn one_event_moves_to_the_evolved_state_and_discovers_it() {
        let machine: Machine = explore(
            vec![Lamp::Off],
            vec![Press::Toggle],
            decide,
            evolve,
            Labels::debug(),
        );
        let expected: Machine = Machine {
            states: vec![state("s_Off", "Off"), state("s_On", "On")],
            transitions: vec![Transition {
                from: String::from("s_Off"),
                to: String::from("s_On"),
                input: String::from("Toggle"),
                commands: vec![String::from("PowerOn"), String::from("Log")],
            }],
        };
        assert_eq!(machine, expected);
    }

    #[test]
    fn many_events_are_folded_in_order() {
        let machine: Machine = explore(
            vec![Lamp::On],
            vec![Press::Toggle, Press::Ignore],
            decide,
            evolve,
            Labels::debug(),
        );
        let expected: Machine = Machine {
            states: vec![state("s_On", "On")],
            transitions: vec![Transition {
                from: String::from("s_On"),
                to: String::from("s_On"),
                input: String::from("Toggle"),
                commands: vec![],
            }],
        };
        assert_eq!(machine, expected);
    }

    fn lit(_: &Lamp) -> String {
        String::from("Lamp")
    }

    fn pressed(_: &Press) -> String {
        String::from("press")
    }

    #[test]
    fn states_and_inputs_are_named_by_their_labels_and_states_with_one_label_merge() {
        let machine: Machine = explore(
            vec![Lamp::Off, Lamp::On],
            vec![Press::Toggle],
            decide,
            evolve,
            Labels {
                state: lit,
                input: pressed,
            },
        );
        let expected: Machine = Machine {
            states: vec![state("s_Lamp", "Lamp")],
            transitions: vec![
                Transition {
                    from: String::from("s_Lamp"),
                    to: String::from("s_Lamp"),
                    input: String::from("press"),
                    commands: vec![String::from("PowerOn"), String::from("Log")],
                },
                Transition {
                    from: String::from("s_Lamp"),
                    to: String::from("s_Lamp"),
                    input: String::from("press"),
                    commands: vec![],
                },
            ],
        };
        assert_eq!(machine, expected);
    }
}
