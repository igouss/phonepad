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

impl Registry {
    fn id<S: Debug>(&mut self, state: &S) -> String {
        let label: String = format!("{state:?}");
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
) -> Machine
where
    S: Debug + Clone,
    I: Debug + Clone,
    C: Debug,
{
    let mut registry: Registry = Registry {
        ids: Ids::default(),
        states: Vec::new(),
    };
    for state in &states {
        registry.id(state);
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
                from: registry.id(state),
                to: registry.id(&target),
                input: format!("{input:?}"),
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
    use super::{Machine, State, Transition, explore};
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
            explore(vec![], vec![Press::Toggle], decide, evolve),
            Machine {
                states: vec![],
                transitions: vec![]
            }
        );
    }

    #[test]
    fn inputs_without_events_produce_no_transitions() {
        let machine: Machine = explore(vec![Lamp::Off], vec![Press::Ignore], decide, evolve);
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
        let machine: Machine = explore(vec![Lamp::Off], vec![Press::Toggle], decide, evolve);
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
}
