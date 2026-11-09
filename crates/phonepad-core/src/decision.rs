#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision<E, C> {
    pub events: Vec<E>,
    pub commands: Vec<C>,
}

impl<E, C> Decision<E, C> {
    pub fn none() -> Decision<E, C> {
        Decision {
            events: Vec::new(),
            commands: Vec::new(),
        }
    }
}
