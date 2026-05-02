use std::{collections::HashMap, hash::Hash, sync::Arc};

use bollard::{Docker, plugin::ContainerSummary};

use crate::{
    error::ProducerError,
    log::{Source, SourceId},
    producer::normalizer::Normalizer,
};

pub struct DockerProducer {
    docker: Arc<Docker>,
    sources: HashMap<SourceId, Source>,
    normalizer: Normalizer,
}

impl DockerProducer {
    pub fn new() -> Result<Self, ProducerError> {
        let docker = Docker::connect_with_defaults()?;

        Ok(DockerProducer::new_seeded(docker))
    }

    pub fn new_seeded(docker: Docker) -> Self {
        DockerProducer {
            docker: Arc::new(docker),
            sources: HashMap::new(),
            normalizer: Normalizer::new(),
        }
    }

    /// List the currently running docker containers that can become log sources.
    async fn list_running_containers(&self) -> Result<Vec<ContainerSummary>, ProducerError> {
        let mut list_container_filters: HashMap<String, Vec<String>> = HashMap::new();
        list_container_filters.insert(String::from("status"), vec![String::from("running")]);

        // Fetch running containers. Explicitly set the filter so that we don't rely on
        // .all(false) returning running containers.
        let containers: Vec<ContainerSummary> = self
            .docker
            .list_containers(Some(
                bollard::query_parameters::ListContainersOptionsBuilder::default()
                    .all(true)
                    .filters(&list_container_filters)
                    .build(),
            ))
            .await?;

        Ok(containers)
    }
}
