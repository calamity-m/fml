use crate::log::Source;

pub struct ProducerState {
    pub sources: Vec<Source>,
}

impl Default for ProducerState {
    fn default() -> Self {
        Self::new()
    }
}

impl ProducerState {
    pub fn new() -> Self {
        ProducerState { sources: vec![] }
    }
}
