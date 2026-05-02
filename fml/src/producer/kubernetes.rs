use std::{collections::HashMap, sync::Arc};

use futures_util::{AsyncBufReadExt, TryStreamExt};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    Api, Client, ResourceExt,
    api::{ListParams, LogParams},
};
use tokio::sync::mpsc;
use tracing::{debug, info};

use crate::{
    error::{FmlError, ProducerError},
    log::{Source, SourceId},
    producer::{LogProducer, normalizer::Normalizer},
};

pub struct KubernetesProducer {
    client: Arc<Client>,
    namespace: String,
    sources: HashMap<SourceId, Source>,

    normalizer: Normalizer,
}

impl KubernetesProducer {
    pub fn new(namespace: String) -> Result<Self, ProducerError> {
        let kubeconfig = kube::config::Kubeconfig::read()?;
        let client = Client::try_from(kubeconfig)?;

        Ok(KubernetesProducer::new_seeded(namespace, client))
    }

    pub fn new_seeded(namespace: String, client: Client) -> KubernetesProducer {
        KubernetesProducer {
            client: Arc::new(client),
            namespace,
            sources: HashMap::new(),
            normalizer: Normalizer::new(),
        }
    }
}

impl LogProducer for KubernetesProducer {
    fn start(&self, tx: mpsc::Sender<crate::event::ProducerEvent>) {
        todo!()
    }

    fn stop(&self) {
        todo!()
    }
}
