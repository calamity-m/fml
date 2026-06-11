//! Docker-backed log producer.
//!
//! `DockerProducer` discovers running containers, tails stdout/stderr from
//! each one, and listens for container start/die/destroy events to keep the
//! source set current. It subscribes to Docker events before listing running
//! containers so containers that start during initialization are picked up by
//! the event stream.

use std::{collections::HashMap, sync::Arc};

#[cfg(test)]
use bollard::API_DEFAULT_VERSION;
use bollard::{
    Docker,
    container::LogOutput,
    plugin::{ContainerSummary, EventMessage},
    query_parameters::{EventsOptionsBuilder, ListContainersOptionsBuilder, LogsOptionsBuilder},
};
use futures_util::StreamExt as _;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

#[cfg(test)]
use crate::producer::file::decode_line;
use crate::{
    config::IngestConfig,
    error::ProducerError,
    event::ProducerEvent,
    log::{Source, SourceId},
    producer::{LogProducer, SourceBlock, file::LineBuffer, normalizer::Normalizer},
};

pub struct DockerProducer {
    docker: Arc<Docker>,
    normalizer: Normalizer,
    cancel: CancellationToken,
    source_block: Arc<SourceBlock>,
    ingest: IngestConfig,
}

impl DockerProducer {
    pub fn new(source_block: SourceBlock, ingest: IngestConfig) -> Result<Self, ProducerError> {
        let docker = Docker::connect_with_local_defaults()?;

        Ok(DockerProducer::new_seeded(docker, source_block, ingest))
    }

    #[cfg(test)]
    fn new_with_socket_path(path: &str) -> Result<Self, ProducerError> {
        let docker = Docker::connect_with_socket(path, 120, API_DEFAULT_VERSION)?;

        Ok(DockerProducer::new_seeded(
            docker,
            SourceBlock::none(),
            IngestConfig::default(),
        ))
    }

    pub fn new_seeded(docker: Docker, source_block: SourceBlock, ingest: IngestConfig) -> Self {
        DockerProducer {
            docker: Arc::new(docker),
            normalizer: Normalizer::new(),
            cancel: CancellationToken::new(),
            source_block: Arc::new(source_block),
            ingest,
        }
    }
}

impl LogProducer for DockerProducer {
    fn start(&self, tx: mpsc::Sender<ProducerEvent>) {
        let docker = self.docker.clone();
        let normalizer = self.normalizer.clone();
        let cancel = self.cancel.clone();
        let source_block = self.source_block.clone();
        let ingest = self.ingest;

        tokio::spawn(async move {
            if let Err(err) =
                run_docker_producer(docker, normalizer, tx, cancel, source_block, ingest).await
            {
                warn!("docker producer exited with error: {err}");
            }
        });
    }

    fn stop(&self) {
        self.cancel.cancel();
    }
}

async fn run_docker_producer(
    docker: Arc<Docker>,
    normalizer: Normalizer,
    tx: mpsc::Sender<ProducerEvent>,
    cancel: CancellationToken,
    source_block: Arc<SourceBlock>,
    ingest: IngestConfig,
) -> Result<(), ProducerError> {
    let (event_tx, mut event_rx) = mpsc::channel(64);
    tokio::spawn(pump_docker_events(docker.clone(), event_tx, cancel.clone()));

    let containers = list_running_containers(&docker).await?;
    let mut tracked: HashMap<SourceId, CancellationToken> = HashMap::new();

    for container in containers {
        let source = container_summary_to_source(&container);
        track_container(
            &mut tracked,
            source,
            docker.clone(),
            normalizer.clone(),
            tx.clone(),
            &cancel,
            &source_block,
            ingest,
        )
        .await;
    }

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            event = event_rx.recv() => {
                let Some(event) = event else {
                    warn!("docker events stream closed; docker producer will stop tracking container churn");
                    break;
                };

                handle_docker_event(
                    event,
                    &mut tracked,
                    docker.clone(),
                    normalizer.clone(),
                    tx.clone(),
                    &cancel,
                    &source_block,
                    ingest,
                ).await;
            }
        }
    }

    for (source_id, child) in tracked {
        child.cancel();
        let _ = tx.send(ProducerEvent::SourceLost(source_id)).await;
    }

    Ok(())
}

async fn pump_docker_events(
    docker: Arc<Docker>,
    tx: mpsc::Sender<EventMessage>,
    cancel: CancellationToken,
) {
    let mut events = docker.events(Some(events_options()));

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            event = events.next() => {
                let Some(event) = event else { break };
                match event {
                    Ok(event) => {
                        if tx.send(event).await.is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        warn!("docker events stream errored: {err}; docker producer will stop tracking container churn");
                        break;
                    }
                }
            }
        }
    }
}

async fn list_running_containers(docker: &Docker) -> Result<Vec<ContainerSummary>, ProducerError> {
    let mut filters: HashMap<String, Vec<String>> = HashMap::new();
    filters.insert("status".to_string(), vec!["running".to_string()]);

    Ok(docker
        .list_containers(Some(
            ListContainersOptionsBuilder::default()
                .filters(&filters)
                .build(),
        ))
        .await?)
}

fn events_options() -> bollard::query_parameters::EventsOptions {
    let mut filters: HashMap<String, Vec<String>> = HashMap::new();
    filters.insert("type".to_string(), vec!["container".to_string()]);
    filters.insert(
        "event".to_string(),
        vec![
            "start".to_string(),
            "die".to_string(),
            "destroy".to_string(),
        ],
    );

    EventsOptionsBuilder::default().filters(&filters).build()
}

#[allow(clippy::too_many_arguments)]
async fn handle_docker_event(
    event: EventMessage,
    tracked: &mut HashMap<SourceId, CancellationToken>,
    docker: Arc<Docker>,
    normalizer: Normalizer,
    tx: mpsc::Sender<ProducerEvent>,
    parent_cancel: &CancellationToken,
    source_block: &SourceBlock,
    ingest: IngestConfig,
) {
    let Some(action) = event.action.as_deref() else {
        return;
    };
    let Some(actor) = event.actor else {
        return;
    };
    let Some(container_id) = actor.id else {
        return;
    };

    match action {
        "start" if !tracked.contains_key(&container_id) => {
            let labels = actor.attributes.unwrap_or_default();
            let name = labels
                .get("name")
                .map(String::as_str)
                .unwrap_or(&container_id);
            let image = labels.get("image").cloned();
            let source = source_from_parts(&container_id, &[name.to_string()], image, &labels);
            track_container(
                tracked,
                source,
                docker,
                normalizer,
                tx,
                parent_cancel,
                source_block,
                ingest,
            )
            .await;
        }
        "die" | "destroy" => {
            if let Some(child) = tracked.remove(&container_id) {
                child.cancel();
                let _ = tx.send(ProducerEvent::SourceLost(container_id)).await;
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
async fn track_container(
    tracked: &mut HashMap<SourceId, CancellationToken>,
    source: Source,
    docker: Arc<Docker>,
    normalizer: Normalizer,
    tx: mpsc::Sender<ProducerEvent>,
    parent_cancel: &CancellationToken,
    source_block: &SourceBlock,
    ingest: IngestConfig,
) {
    if tracked.contains_key(&source.id) {
        return;
    }

    // Single source-emission gate. The docker producer has exactly one
    // SourceFound emission site (this function); blocked containers are
    // dropped here and never reach the event bus, the tracked map, or
    // the per-container log-tail task.
    if source_block.is_source_blocked(&source) {
        debug!(
            "docker producer source `{}` blocked by SourceBlock; skipping",
            source.id
        );
        return;
    }

    if tx
        .send(ProducerEvent::SourceFound(source.clone()))
        .await
        .is_err()
    {
        debug!(
            "docker producer {} aborting: event channel closed",
            source.id
        );
        return;
    }

    let child = parent_cancel.child_token();
    tokio::spawn(tail_container(
        docker,
        source.id.clone(),
        source.clone(),
        tx,
        normalizer,
        child.clone(),
        ingest,
    ));
    tracked.insert(source.id, child);
}

/// Build the log options for one container tail.
///
/// With backfill enabled this is a single `follow(true)` stream that opens
/// with bounded history: the daemon selects the last `tail` lines, filters
/// them to those at or after `since`, returns that history in stream order,
/// then keeps streaming live output. One stream means there is no
/// backfill-to-live handoff gap. Verified against the Docker Engine API
/// (`GET /containers/{id}/logs`) semantics for bollard 0.20 / API 1.52:
/// `tail` and `since` compose as select-then-filter and both apply before
/// `follow` continues the stream.
///
/// With backfill disabled (`backfill_max_lines_per_source == 0`) this
/// preserves the previous live-only behavior: `tail("0")` skips all existing
/// output.
fn tail_log_options(ingest: IngestConfig, now_secs: i64) -> bollard::query_parameters::LogsOptions {
    let builder = LogsOptionsBuilder::default()
        .follow(true)
        .stdout(true)
        .stderr(true);

    if ingest.backfill_enabled() {
        // bollard models `since` as i32 seconds; saturate rather than wrap
        // for absurd windows.
        let since = now_secs.saturating_sub(ingest.backfill_window_secs as i64);
        builder
            .since(since.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
            .tail(&ingest.backfill_max_lines_per_source.to_string())
            .build()
    } else {
        builder.tail("0").build()
    }
}

async fn tail_container(
    docker: Arc<Docker>,
    container_id: SourceId,
    source: Source,
    tx: mpsc::Sender<ProducerEvent>,
    normalizer: Normalizer,
    cancel: CancellationToken,
    ingest: IngestConfig,
) {
    let mut logs = docker.logs(
        &container_id,
        Some(tail_log_options(ingest, chrono::Utc::now().timestamp())),
    );
    let mut buffer = LineBuffer::default();

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            item = logs.next() => {
                let Some(item) = item else { break };
                match item {
                    Ok(output) => {
                        for line in buffer.push(output_bytes(output).as_ref()) {
                            let entry = normalizer.normalize(&line, source.clone());
                            if tx.send(ProducerEvent::StoreEvent(entry)).await.is_err() {
                                debug!("docker tail {container_id} aborting: event channel closed");
                                return;
                            }
                        }
                    }
                    Err(err) => {
                        warn!("docker logs stream errored for {container_id}: {err}");
                        break;
                    }
                }
            }
        }
    }
}

fn output_bytes(output: LogOutput) -> bytes::Bytes {
    match output {
        LogOutput::StdOut { message }
        | LogOutput::StdErr { message }
        | LogOutput::Console { message }
        | LogOutput::StdIn { message } => message,
    }
}

fn container_summary_to_source(container: &ContainerSummary) -> Source {
    let id = container.id.clone().unwrap_or_default();
    let names = container.names.as_deref().unwrap_or(&[]);
    let labels = container.labels.as_ref().cloned().unwrap_or_default();
    source_from_parts(&id, names, container.image.clone(), &labels)
}

fn source_from_parts(
    id: &str,
    names: &[String],
    image: Option<String>,
    labels: &HashMap<String, String>,
) -> Source {
    let display_name = names
        .first()
        .map(|name| name.trim_start_matches('/').to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| short_container_id(id));
    let group = labels
        .get("com.docker.compose.project")
        .cloned()
        .or(image)
        .filter(|group| !group.is_empty());

    Source {
        producer: "docker".to_string(),
        id: id.to_string(),
        display_name,
        group,
    }
}

fn short_container_id(id: &str) -> String {
    id.chars().take(12).collect()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn summary(
        id: &str,
        names: Option<Vec<&str>>,
        image: Option<&str>,
        labels: HashMap<String, String>,
    ) -> ContainerSummary {
        ContainerSummary {
            id: Some(id.to_string()),
            names: names.map(|names| names.into_iter().map(str::to_string).collect()),
            image: image.map(str::to_string),
            labels: Some(labels),
            ..Default::default()
        }
    }

    #[test]
    fn source_uses_compose_label_as_group() {
        let mut labels = HashMap::new();
        labels.insert(
            "com.docker.compose.project".to_string(),
            "my-project".to_string(),
        );
        let source = container_summary_to_source(&summary(
            "0123456789abcdef",
            Some(vec!["/web"]),
            Some("nginx:latest"),
            labels,
        ));

        assert_eq!(source.producer, "docker");
        assert_eq!(source.id, "0123456789abcdef");
        assert_eq!(source.display_name, "web");
        assert_eq!(source.group, Some("my-project".to_string()));
    }

    #[test]
    fn source_uses_image_as_group_without_compose_label() {
        let source = container_summary_to_source(&summary(
            "0123456789abcdef",
            Some(vec!["/api"]),
            Some("busybox:latest"),
            HashMap::new(),
        ));

        assert_eq!(source.group, Some("busybox:latest".to_string()));
    }

    #[test]
    fn source_uses_first_container_name() {
        let source = container_summary_to_source(&summary(
            "0123456789abcdef",
            Some(vec!["/first", "/second"]),
            None,
            HashMap::new(),
        ));

        assert_eq!(source.display_name, "first");
    }

    #[test]
    fn source_strips_leading_slash_from_name() {
        let source = container_summary_to_source(&summary(
            "0123456789abcdef",
            Some(vec!["/worker"]),
            None,
            HashMap::new(),
        ));

        assert_eq!(source.display_name, "worker");
    }

    #[test]
    fn source_falls_back_to_short_id_without_names() {
        let source =
            container_summary_to_source(&summary("0123456789abcdef", None, None, HashMap::new()));

        assert_eq!(source.display_name, "0123456789ab");
    }

    #[test]
    fn new_with_bogus_socket_path_errors() {
        let err = match DockerProducer::new_with_socket_path("/tmp/fml-missing-docker.sock") {
            Ok(_) => panic!("bogus socket should error"),
            Err(err) => err,
        };

        assert!(matches!(err, ProducerError::Docker(_)));
    }

    #[tokio::test]
    async fn start_returns_promptly_when_docker_api_is_unreachable() {
        let docker = Docker::connect_with_http("127.0.0.1:9", 1, API_DEFAULT_VERSION)
            .expect("http docker handle");
        let producer =
            DockerProducer::new_seeded(docker, SourceBlock::none(), IngestConfig::default());
        let (tx, _rx) = mpsc::channel(8);

        let start = Instant::now();
        producer.start(tx);

        assert!(start.elapsed() < Duration::from_millis(50));
        producer.stop();
    }

    #[test]
    fn decode_line_helper_is_available_for_docker_tail() {
        assert_eq!(decode_line(b"docker \xFF"), "docker �");
    }

    fn unreachable_docker() -> Arc<Docker> {
        Arc::new(
            Docker::connect_with_http("127.0.0.1:9", 1, API_DEFAULT_VERSION)
                .expect("http docker handle"),
        )
    }

    #[tokio::test]
    async fn track_container_skips_blocked_source_via_regex() {
        use crate::config::SourceBlockConfig;

        let docker = unreachable_docker();
        let normalizer = Normalizer::new();
        let cancel = CancellationToken::new();
        let block = SourceBlock::from_config(
            &SourceBlockConfig {
                blocked: Some("postgres".to_string()),
                skip_istio: false,
            },
            false,
        )
        .unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        let mut tracked: HashMap<SourceId, CancellationToken> = HashMap::new();

        let source = Source {
            producer: "docker".to_string(),
            id: "abc123".to_string(),
            display_name: "team_postgres_1".to_string(),
            group: None,
        };

        track_container(
            &mut tracked,
            source,
            docker,
            normalizer,
            tx,
            &cancel,
            &block,
            IngestConfig::default(),
        )
        .await;

        cancel.cancel();
        assert!(tracked.is_empty());
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn track_container_skips_blocked_source_via_skip_istio() {
        use crate::config::SourceBlockConfig;

        let docker = unreachable_docker();
        let normalizer = Normalizer::new();
        let cancel = CancellationToken::new();
        let block = SourceBlock::from_config(
            &SourceBlockConfig {
                blocked: None,
                skip_istio: true,
            },
            false,
        )
        .unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        let mut tracked: HashMap<SourceId, CancellationToken> = HashMap::new();

        let source = Source {
            producer: "docker".to_string(),
            id: "abc123".to_string(),
            display_name: "istio-proxy".to_string(),
            group: None,
        };

        track_container(
            &mut tracked,
            source,
            docker,
            normalizer,
            tx,
            &cancel,
            &block,
            IngestConfig::default(),
        )
        .await;

        cancel.cancel();
        assert!(tracked.is_empty());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn tail_log_options_with_backfill_sets_since_and_tail_cap() {
        let ingest = IngestConfig {
            backfill_window_secs: 1800,
            backfill_max_lines_per_source: 5000,
        };

        let options = tail_log_options(ingest, 1_000_000);

        assert!(options.follow);
        assert!(options.stdout);
        assert!(options.stderr);
        assert_eq!(options.since, 1_000_000 - 1800);
        assert_eq!(options.tail, "5000");
    }

    #[test]
    fn tail_log_options_disabled_backfill_keeps_live_only_tail_zero() {
        let ingest = IngestConfig {
            backfill_max_lines_per_source: 0,
            ..IngestConfig::default()
        };

        let options = tail_log_options(ingest, 1_000_000);

        assert!(options.follow);
        assert_eq!(options.tail, "0");
        // Default `since` of 0 means no server-side time filtering.
        assert_eq!(options.since, 0);
    }
}
