use tokio::sync::mpsc;

use crate::{event::ProducerEvent, log::SourceId, producer::LogProducer};

pub struct FakeProducer {
    source_id: SourceId,
}

impl FakeProducer {
    pub fn new(source_id: SourceId) -> Self {
        Self { source_id }
    }
}

impl LogProducer for FakeProducer {
    fn source_id(&self) -> SourceId {
        self.source_id.clone()
    }

    fn start(&self, _tx: mpsc::Sender<ProducerEvent>) {
        todo!()
    }

    fn stop(&self) {
        todo!()
    }
}
