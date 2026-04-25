use crate::log::Source;

pub struct ProducerState {
    pub sources: Vec<Source>,
}

impl ProducerState {
    pub fn new() -> Self {
        ProducerState { sources: vec![] }
    }
}
