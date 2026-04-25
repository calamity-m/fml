use crate::producer::LogProducer;

pub struct FakeProducer {}

impl LogProducer for FakeProducer {
    fn start(&self, tx: tokio::sync::mpsc::Sender<crate::event::ProducerEvent>) {
        todo!()
    }

    fn stop(&self) {
        todo!()
    }
}
