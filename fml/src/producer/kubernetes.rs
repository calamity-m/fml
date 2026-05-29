//! Kubernetes-backed log producer.
//!
//! `KubernetesProducer` watches pods in one namespace and tails each running
//! regular or init container as its own source. It reads kubeconfig from the
//! standard local locations; in-cluster service-account configs, explicit
//! `--context` overrides, and multi-cluster federation are out of scope for
//! this producer.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use futures_util::{AsyncBufReadExt as _, StreamExt as _, TryStreamExt as _};
use k8s_openapi::api::core::v1::{ContainerStatus, Pod};
use kube::{
    Api, Client, ResourceExt,
    api::LogParams,
    config::{Config, Kubeconfig},
    runtime::watcher::{self, Event},
};
use tokio::{sync::mpsc, time::sleep};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::{
    error::ProducerError,
    event::ProducerEvent,
    log::{Source, SourceId},
    producer::{LogProducer, SourceBlock, file::decode_line, normalizer::Normalizer},
};

type ContainerKey = (String, String);

/// Discovers and tails running containers in one Kubernetes namespace.
pub struct KubernetesProducer {
    client: Arc<Client>,
    namespace: String,
    normalizer: Normalizer,
    cancel: CancellationToken,
    source_block: Arc<SourceBlock>,
}

impl KubernetesProducer {
    /// Create a producer using the local kubeconfig and the supplied namespace.
    pub fn new(namespace: String, source_block: SourceBlock) -> Result<Self, ProducerError> {
        let kubeconfig = Kubeconfig::read()?;
        let mut config = Config::try_from(kubeconfig)?;
        apply_no_proxy(&mut config);
        let client = Client::try_from(config)?;

        Ok(KubernetesProducer::new_seeded(
            namespace,
            client,
            source_block,
        ))
    }

    /// Create a producer from an already-constructed Kubernetes client.
    pub fn new_seeded(
        namespace: String,
        client: Client,
        source_block: SourceBlock,
    ) -> KubernetesProducer {
        KubernetesProducer {
            client: Arc::new(client),
            namespace,
            normalizer: Normalizer::new(),
            cancel: CancellationToken::new(),
            source_block: Arc::new(source_block),
        }
    }

    /// Resolve the active kubeconfig context namespace.
    pub fn resolve_namespace() -> Result<String, ProducerError> {
        resolve_namespace_from_kubeconfig(&Kubeconfig::read()?)
    }
}

impl LogProducer for KubernetesProducer {
    fn start(&self, tx: mpsc::Sender<ProducerEvent>) {
        let client = self.client.clone();
        let namespace = self.namespace.clone();
        let normalizer = self.normalizer.clone();
        let cancel = self.cancel.clone();
        let source_block = self.source_block.clone();

        tokio::spawn(async move {
            if let Err(err) =
                run_kubernetes_producer(client, namespace, normalizer, tx, cancel, source_block)
                    .await
            {
                warn!("kubernetes producer exited with error: {err}");
            }
        });
    }

    fn stop(&self) {
        self.cancel.cancel();
    }
}

async fn run_kubernetes_producer(
    client: Arc<Client>,
    namespace: String,
    normalizer: Normalizer,
    tx: mpsc::Sender<ProducerEvent>,
    cancel: CancellationToken,
    source_block: Arc<SourceBlock>,
) -> Result<(), ProducerError> {
    let pods: Api<Pod> = Api::namespaced((*client).clone(), &namespace);
    let mut events = watcher::watcher(pods.clone(), watcher::Config::default()).boxed();
    let mut tracked: HashMap<ContainerKey, CancellationToken> = HashMap::new();
    let mut init_buffer: Vec<Pod> = Vec::new();

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            event = events.next() => {
                let Some(event) = event else { break };
                match event {
                    Ok(Event::Apply(pod)) => {
                        track_running_containers(&pod, &namespace, &pods, &normalizer, &tx, &cancel, &mut tracked, &source_block).await;
                    }
                    Ok(Event::Delete(pod)) => {
                        untrack_pod(&pod.name_any(), &namespace, &tx, &mut tracked).await;
                    }
                    Ok(Event::Init) => init_buffer.clear(),
                    Ok(Event::InitApply(pod)) => init_buffer.push(pod),
                    Ok(Event::InitDone) => {
                        let (additions, removals) = reconcile_restarted(&init_buffer, &mut tracked);
                        for (key, child) in removals {
                            child.cancel();
                            let _ = tx.send(ProducerEvent::SourceLost(source_id_for_key(&namespace, &key))).await;
                        }
                        for running in additions {
                            track_container(running, &namespace, &pods, &normalizer, &tx, &cancel, &mut tracked, &source_block).await;
                        }
                    }
                    Err(err) => warn!("kubernetes watcher error in namespace {namespace}: {err}"),
                }
            }
        }
    }

    for (key, child) in tracked {
        child.cancel();
        let _ = tx
            .send(ProducerEvent::SourceLost(source_id_for_key(
                &namespace, &key,
            )))
            .await;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn track_running_containers(
    pod: &Pod,
    namespace: &str,
    api: &Api<Pod>,
    normalizer: &Normalizer,
    tx: &mpsc::Sender<ProducerEvent>,
    parent_cancel: &CancellationToken,
    tracked: &mut HashMap<ContainerKey, CancellationToken>,
    source_block: &SourceBlock,
) {
    let pod_name = pod.name_any();
    let running = running_containers(pod);
    let running_keys = running
        .iter()
        .map(RunningContainer::key)
        .collect::<HashSet<_>>();
    let stopped_keys = tracked
        .keys()
        .filter(|(tracked_pod, _)| tracked_pod == &pod_name)
        .filter(|key| !running_keys.contains(*key))
        .cloned()
        .collect::<Vec<_>>();

    for key in stopped_keys {
        if let Some(child) = tracked.remove(&key) {
            child.cancel();
            let _ = tx
                .send(ProducerEvent::SourceLost(source_id_for_key(
                    namespace, &key,
                )))
                .await;
        }
    }

    for running in running {
        track_container(
            running,
            namespace,
            api,
            normalizer,
            tx,
            parent_cancel,
            tracked,
            source_block,
        )
        .await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn track_container(
    running: RunningContainer,
    namespace: &str,
    api: &Api<Pod>,
    normalizer: &Normalizer,
    tx: &mpsc::Sender<ProducerEvent>,
    parent_cancel: &CancellationToken,
    tracked: &mut HashMap<ContainerKey, CancellationToken>,
    source_block: &SourceBlock,
) {
    let key = running.key();
    if tracked.contains_key(&key) {
        return;
    }

    let source = pod_container_to_source(&running, namespace);

    // Single source-emission gate. The kubernetes producer has exactly one
    // SourceFound emission site (this function); blocked sources are dropped
    // here, so the per-pod log-tail task is never spawned and no
    // StoreEvent is ever forwarded for them.
    if source_block.is_source_blocked(&source) {
        debug!(
            "kubernetes producer source `{}` blocked by SourceBlock; skipping",
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
            "kubernetes producer {} aborting: event channel closed",
            source.id
        );
        return;
    }

    let child = parent_cancel.child_token();
    tokio::spawn(tail_pod_container(
        api.clone(),
        key.clone(),
        source,
        tx.clone(),
        *normalizer,
        child.clone(),
    ));
    tracked.insert(key, child);
}

async fn untrack_pod(
    pod_name: &str,
    namespace: &str,
    tx: &mpsc::Sender<ProducerEvent>,
    tracked: &mut HashMap<ContainerKey, CancellationToken>,
) {
    let removed = tracked
        .keys()
        .filter(|(tracked_pod, _)| tracked_pod == pod_name)
        .cloned()
        .collect::<Vec<_>>();

    for key in removed {
        if let Some(child) = tracked.remove(&key) {
            child.cancel();
            let _ = tx
                .send(ProducerEvent::SourceLost(source_id_for_key(
                    namespace, &key,
                )))
                .await;
        }
    }
}

async fn tail_pod_container(
    api: Api<Pod>,
    key: ContainerKey,
    source: Source,
    tx: mpsc::Sender<ProducerEvent>,
    normalizer: Normalizer,
    cancel: CancellationToken,
) {
    let mut backoff = ReconnectBackoff::new();

    loop {
        if cancel.is_cancelled() {
            break;
        }

        let params = LogParams {
            container: Some(key.1.clone()),
            follow: true,
            tail_lines: Some(0),
            ..LogParams::default()
        };

        match api.log_stream(&key.0, &params).await {
            Ok(stream) => {
                let mut lines = stream.lines();
                let mut read_any = false;
                loop {
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => return,
                        line = lines.try_next() => {
                            match line {
                                Ok(Some(line)) => {
                                    if !read_any {
                                        backoff.reset();
                                    }
                                    read_any = true;
                                    let line = decode_line(line.as_bytes());
                                    let entry = normalizer.normalize(&line, source.clone());
                                    if tx.send(ProducerEvent::StoreEvent(entry)).await.is_err() {
                                        debug!("kubernetes tail {} aborting: event channel closed", source.id);
                                        return;
                                    }
                                }
                                Ok(None) => break,
                                Err(err) => {
                                    warn!("kubernetes log stream errored for {}: {err}", source.id);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            Err(err) => warn!("kubernetes log_stream open failed for {}: {err}", source.id),
        }

        let delay = backoff.delay();
        warn!(
            "kubernetes tail for {} will reconnect in {:?}; catch-up across the disconnected window is not guaranteed",
            source.id, delay
        );
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = sleep(delay) => backoff.advance(),
        }
    }
}

fn reconcile_restarted(
    observed: &[Pod],
    tracked: &mut HashMap<ContainerKey, CancellationToken>,
) -> (
    Vec<RunningContainer>,
    Vec<(ContainerKey, CancellationToken)>,
) {
    let observed = observed
        .iter()
        .flat_map(running_containers)
        .collect::<Vec<_>>();
    let observed_keys = observed
        .iter()
        .map(RunningContainer::key)
        .collect::<HashSet<_>>();

    let removed_keys = tracked
        .keys()
        .filter(|key| !observed_keys.contains(*key))
        .cloned()
        .collect::<Vec<_>>();
    let removals = removed_keys
        .into_iter()
        .filter_map(|key| tracked.remove(&key).map(|token| (key, token)))
        .collect::<Vec<_>>();

    let additions = observed
        .into_iter()
        .filter(|running| !tracked.contains_key(&running.key()))
        .collect();

    (additions, removals)
}

fn running_containers(pod: &Pod) -> Vec<RunningContainer> {
    let pod_name = pod.name_any();
    let workload_group = pod_workload_group(pod);
    let mut containers = Vec::new();
    if let Some(status) = &pod.status {
        collect_running(
            &pod_name,
            status.container_statuses.as_deref(),
            &workload_group,
            &mut containers,
        );
        collect_running(
            &pod_name,
            status.init_container_statuses.as_deref(),
            &workload_group,
            &mut containers,
        );
    }
    containers
}

fn collect_running(
    pod_name: &str,
    statuses: Option<&[ContainerStatus]>,
    workload_group: &str,
    containers: &mut Vec<RunningContainer>,
) {
    for status in statuses.unwrap_or_default() {
        if status
            .state
            .as_ref()
            .and_then(|state| state.running.as_ref())
            .is_some()
        {
            containers.push(RunningContainer {
                pod_name: pod_name.to_string(),
                container_name: status.name.clone(),
                workload_group: workload_group.to_string(),
            });
        }
    }
}

fn pod_container_to_source(running: &RunningContainer, namespace: &str) -> Source {
    Source {
        producer: namespace.to_string(),
        id: format!(
            "{namespace}/{}/{}",
            running.pod_name, running.container_name
        ),
        display_name: format!("{}/{}", running.pod_name, running.container_name),
        group: Some(running.workload_group.clone()),
    }
}

fn pod_workload_group(pod: &Pod) -> String {
    let pod_name = pod.name_any();
    pod.metadata
        .owner_references
        .as_deref()
        .and_then(|owners| {
            owners
                .iter()
                .find(|owner| owner.controller.unwrap_or(false))
                .or_else(|| owners.first())
        })
        .map(|owner| workload_group_for_owner(&owner.kind, &owner.name))
        .unwrap_or_else(|| format!("pod/{pod_name}"))
}

fn workload_group_for_owner(kind: &str, name: &str) -> String {
    let kind = kind.to_ascii_lowercase();
    if kind == "replicaset"
        && let Some(deployment) = deployment_name_from_replicaset(name)
    {
        return format!("deployment/{deployment}");
    }

    format!("{kind}/{name}")
}

fn deployment_name_from_replicaset(name: &str) -> Option<&str> {
    let (deployment, suffix) = name.rsplit_once('-')?;
    let generated_hash = (5..=10).contains(&suffix.len())
        && suffix
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit());
    generated_hash
        .then_some(deployment)
        .filter(|s| !s.is_empty())
}

fn source_id_for_key(namespace: &str, key: &ContainerKey) -> SourceId {
    format!("{namespace}/{}/{}", key.0, key.1)
}

/// Clear `config.proxy_url` when the cluster host matches the `NO_PROXY`
/// environment variable.
///
/// kube 3.1 resolves `HTTPS_PROXY` but ignores `NO_PROXY` (kube-rs/kube#1203),
/// so a configured proxy is applied even to clusters meant to be reached
/// directly. We strip the proxy here before the client is built.
fn apply_no_proxy(config: &mut Config) {
    if config.proxy_url.is_none() {
        return;
    }
    let no_proxy = std::env::var("NO_PROXY")
        .or_else(|_| std::env::var("no_proxy"))
        .unwrap_or_default();
    if let Some(host) = config.cluster_url.host()
        && host_bypasses_proxy(host, &no_proxy)
    {
        config.proxy_url = None;
    }
}

/// Returns true if `host` should bypass the proxy per a `NO_PROXY`-style value.
///
/// `no_proxy` is a comma-separated list. Matching follows the common
/// Go/curl convention: `*` bypasses every host, and each entry matches `host`
/// either exactly or as a domain suffix (`example.com` matches
/// `api.example.com`). CIDR ranges are not handled; an IP literal only matches
/// the same literal verbatim.
fn host_bypasses_proxy(host: &str, no_proxy: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    no_proxy.split(',').any(|raw| {
        let entry = raw.trim().trim_matches('.').to_ascii_lowercase();
        if entry.is_empty() {
            return false;
        }
        if entry == "*" {
            return true;
        }
        host == entry || host.ends_with(&format!(".{entry}"))
    })
}

fn resolve_namespace_from_kubeconfig(kubeconfig: &Kubeconfig) -> Result<String, ProducerError> {
    let active = kubeconfig
        .current_context
        .as_deref()
        .ok_or_else(|| ProducerError::Kubernetes("kubeconfig has no active context".to_string()))?;
    let context = kubeconfig
        .contexts
        .iter()
        .find(|context| context.name == active)
        .ok_or_else(|| {
            ProducerError::Kubernetes(format!(
                "kubeconfig active context `{active}` was not found"
            ))
        })?;

    Ok(context
        .context
        .as_ref()
        .and_then(|context| context.namespace.clone())
        .unwrap_or_else(|| "default".to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunningContainer {
    pod_name: String,
    container_name: String,
    workload_group: String,
}

impl RunningContainer {
    fn key(&self) -> ContainerKey {
        (self.pod_name.clone(), self.container_name.clone())
    }
}

#[derive(Debug)]
struct ReconnectBackoff {
    current: Duration,
}

impl ReconnectBackoff {
    fn new() -> Self {
        Self {
            current: Duration::from_millis(100),
        }
    }

    fn delay(&self) -> Duration {
        self.current
    }

    fn advance(&mut self) {
        self.current = (self.current * 10).min(Duration::from_secs(10));
    }

    fn reset(&mut self) {
        self.current = Duration::from_millis(100);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use k8s_openapi::{
        api::core::v1::{ContainerState, ContainerStateRunning, PodStatus},
        apimachinery::pkg::apis::meta::v1::OwnerReference,
    };
    use kube::{
        Config,
        config::{Context, NamedContext},
    };

    use super::*;
    use crate::config::SourceBlockConfig;

    fn pod(name: &str, regular: &[&str], init: &[&str]) -> Pod {
        pod_with_owner(name, regular, init, None)
    }

    fn pod_with_owner(
        name: &str,
        regular: &[&str],
        init: &[&str],
        owner: Option<(&str, &str)>,
    ) -> Pod {
        Pod {
            metadata: kube::core::ObjectMeta {
                name: Some(name.to_string()),
                owner_references: owner.map(|(kind, name)| {
                    vec![OwnerReference {
                        kind: kind.to_string(),
                        name: name.to_string(),
                        controller: Some(true),
                        ..Default::default()
                    }]
                }),
                ..Default::default()
            },
            status: Some(PodStatus {
                container_statuses: Some(regular.iter().map(|name| status(name, true)).collect()),
                init_container_statuses: Some(init.iter().map(|name| status(name, true)).collect()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn running(pod_name: &str, container_name: &str) -> RunningContainer {
        RunningContainer {
            pod_name: pod_name.to_string(),
            container_name: container_name.to_string(),
            workload_group: format!("pod/{pod_name}"),
        }
    }

    fn status(name: &str, running: bool) -> ContainerStatus {
        ContainerStatus {
            name: name.to_string(),
            state: Some(ContainerState {
                running: running.then(ContainerStateRunning::default),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn token_map(keys: &[(&str, &str)]) -> HashMap<ContainerKey, CancellationToken> {
        keys.iter()
            .map(|(pod, container)| {
                (
                    ((*pod).to_string(), (*container).to_string()),
                    CancellationToken::new(),
                )
            })
            .collect()
    }

    fn kubeconfig(active: Option<&str>, contexts: Vec<NamedContext>) -> Kubeconfig {
        Kubeconfig {
            current_context: active.map(str::to_string),
            contexts,
            ..Default::default()
        }
    }

    fn context(name: &str, namespace: Option<&str>) -> NamedContext {
        NamedContext {
            name: name.to_string(),
            context: Some(Context {
                cluster: "cluster".to_string(),
                namespace: namespace.map(str::to_string),
                ..Default::default()
            }),
        }
    }

    #[test]
    fn pod_container_source_uses_namespace_workload_pod_and_container() {
        let source = pod_container_to_source(
            &RunningContainer {
                pod_name: "web-123".to_string(),
                container_name: "nginx".to_string(),
                workload_group: "deployment/web".to_string(),
            },
            "prod",
        );

        assert_eq!(source.producer, "prod");
        assert_eq!(source.id, "prod/web-123/nginx");
        assert_eq!(source.display_name, "web-123/nginx");
        assert_eq!(source.group, Some("deployment/web".to_string()));
    }

    #[test]
    fn running_containers_group_bare_pods_by_pod_name() {
        let containers = running_containers(&pod("debug", &["shell"], &[]));

        assert_eq!(containers, vec![running("debug", "shell")]);
    }

    #[test]
    fn running_containers_group_by_controller_owner() {
        let containers = running_containers(&pod_with_owner(
            "redis-0",
            &["redis"],
            &[],
            Some(("StatefulSet", "redis")),
        ));

        assert_eq!(containers[0].workload_group, "statefulset/redis");
    }

    #[test]
    fn running_containers_resolve_replicaset_owner_to_deployment() {
        let containers = running_containers(&pod_with_owner(
            "fastapi-server-7df78c6b8c-abc12",
            &["fastapi-server"],
            &[],
            Some(("ReplicaSet", "fastapi-server-7df78c6b8c")),
        ));

        assert_eq!(containers[0].workload_group, "deployment/fastapi-server");
    }

    #[test]
    fn resolve_namespace_uses_active_context_namespace() {
        let namespace = resolve_namespace_from_kubeconfig(&kubeconfig(
            Some("ctx"),
            vec![context("ctx", Some("apps"))],
        ))
        .unwrap();

        assert_eq!(namespace, "apps");
    }

    #[test]
    fn resolve_namespace_defaults_when_context_has_no_namespace() {
        let namespace =
            resolve_namespace_from_kubeconfig(&kubeconfig(Some("ctx"), vec![context("ctx", None)]))
                .unwrap();

        assert_eq!(namespace, "default");
    }

    #[test]
    fn resolve_namespace_errors_when_active_context_is_missing() {
        let err = resolve_namespace_from_kubeconfig(&kubeconfig(
            Some("missing"),
            vec![context("ctx", None)],
        ))
        .unwrap_err();

        assert!(matches!(err, ProducerError::Kubernetes(_)));
    }

    #[test]
    fn reconcile_empty_observed_removes_everything() {
        let mut tracked = token_map(&[("pod-a", "web"), ("pod-b", "api")]);

        let (additions, removals) = reconcile_restarted(&[], &mut tracked);

        assert!(additions.is_empty());
        assert_eq!(removals.len(), 2);
        assert!(tracked.is_empty());
    }

    #[test]
    fn reconcile_partial_overlap_adds_and_removes() {
        let mut tracked = token_map(&[("pod-a", "web"), ("pod-b", "api")]);

        let (additions, removals) =
            reconcile_restarted(&[pod("pod-a", &["web", "sidecar"], &[])], &mut tracked);

        assert_eq!(additions, vec![running("pod-a", "sidecar")]);
        assert_eq!(removals.len(), 1);
        assert_eq!(removals[0].0, ("pod-b".to_string(), "api".to_string()));
        assert!(tracked.contains_key(&("pod-a".to_string(), "web".to_string())));
    }

    #[test]
    fn reconcile_full_overlap_has_no_changes() {
        let mut tracked = token_map(&[("pod-a", "web")]);

        let (additions, removals) =
            reconcile_restarted(&[pod("pod-a", &["web"], &[])], &mut tracked);

        assert!(additions.is_empty());
        assert!(removals.is_empty());
        assert_eq!(tracked.len(), 1);
    }

    #[test]
    fn reconcile_all_new_returns_additions() {
        let mut tracked = HashMap::new();

        let (additions, removals) =
            reconcile_restarted(&[pod("pod-a", &["web"], &["init"])], &mut tracked);

        assert_eq!(additions.len(), 2);
        assert!(removals.is_empty());
    }

    #[test]
    fn reconcile_mix_of_additions_and_removals() {
        let mut tracked = token_map(&[("pod-a", "old"), ("pod-b", "api")]);

        let (additions, removals) = reconcile_restarted(
            &[pod("pod-a", &["web"], &[]), pod("pod-b", &["api"], &[])],
            &mut tracked,
        );

        assert_eq!(additions, vec![running("pod-a", "web")]);
        assert_eq!(removals.len(), 1);
        assert_eq!(removals[0].0, ("pod-a".to_string(), "old".to_string()));
    }

    #[test]
    fn backoff_advances_caps_and_resets() {
        let mut backoff = ReconnectBackoff::new();

        assert_eq!(backoff.delay(), Duration::from_millis(100));
        backoff.advance();
        assert_eq!(backoff.delay(), Duration::from_secs(1));
        backoff.advance();
        assert_eq!(backoff.delay(), Duration::from_secs(10));
        backoff.advance();
        assert_eq!(backoff.delay(), Duration::from_secs(10));
        backoff.reset();
        assert_eq!(backoff.delay(), Duration::from_millis(100));
    }

    fn unreachable_api() -> Api<Pod> {
        let config = Config::new("http://127.0.0.1:9".parse().unwrap());
        let client = Client::try_from(config).expect("test kube client");
        Api::namespaced(client, "default")
    }

    #[tokio::test]
    async fn track_container_skips_blocked_source_by_display_name() {
        let api = unreachable_api();
        let cancel = CancellationToken::new();
        let normalizer = Normalizer::new();
        let block = SourceBlock::from_config(
            &SourceBlockConfig {
                blocked: Some("^istio".to_string()),
                skip_istio: false,
            },
            false,
        )
        .unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        let mut tracked: HashMap<ContainerKey, CancellationToken> = HashMap::new();

        track_container(
            RunningContainer {
                pod_name: "productpage".to_string(),
                container_name: "istio-proxy".to_string(),
                workload_group: "deployment/productpage".to_string(),
            },
            "default",
            &api,
            &normalizer,
            &tx,
            &cancel,
            &mut tracked,
            &block,
        )
        .await;

        cancel.cancel();
        assert!(tracked.is_empty());
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn track_container_skips_blocked_source_via_skip_istio() {
        let api = unreachable_api();
        let cancel = CancellationToken::new();
        let normalizer = Normalizer::new();
        let block = SourceBlock::from_config(
            &SourceBlockConfig {
                blocked: None,
                skip_istio: true,
            },
            false,
        )
        .unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        let mut tracked: HashMap<ContainerKey, CancellationToken> = HashMap::new();

        // Display name includes the container name, e.g. `productpage/istio-proxy`.
        track_container(
            RunningContainer {
                pod_name: "productpage".to_string(),
                container_name: "istio-proxy".to_string(),
                workload_group: "deployment/productpage".to_string(),
            },
            "default",
            &api,
            &normalizer,
            &tx,
            &cancel,
            &mut tracked,
            &block,
        )
        .await;

        cancel.cancel();
        assert!(tracked.is_empty());
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn track_container_does_not_skip_unrelated_source() {
        // Sanity: with a matching block, a non-matching pod still announces.
        let api = unreachable_api();
        let cancel = CancellationToken::new();
        let normalizer = Normalizer::new();
        let block = SourceBlock::from_config(
            &SourceBlockConfig {
                blocked: Some("^istio".to_string()),
                skip_istio: false,
            },
            false,
        )
        .unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        let mut tracked: HashMap<ContainerKey, CancellationToken> = HashMap::new();

        track_container(
            RunningContainer {
                pod_name: "api".to_string(),
                container_name: "web".to_string(),
                workload_group: "deployment/api".to_string(),
            },
            "default",
            &api,
            &normalizer,
            &tx,
            &cancel,
            &mut tracked,
            &block,
        )
        .await;

        // SourceFound was emitted; cancel before the spawned tail can do work.
        cancel.cancel();
        let evt = rx.try_recv().expect("expected SourceFound");
        match evt {
            ProducerEvent::SourceFound(s) => {
                assert_eq!(s.display_name, "api/web");
            }
            other => panic!("unexpected event: {other:?}"),
        }
        assert_eq!(tracked.len(), 1);
    }

    #[tokio::test]
    async fn start_returns_promptly_when_kube_api_is_unreachable() {
        let config = Config::new("http://127.0.0.1:9".parse().unwrap());
        let client = Client::try_from(config).expect("test kube client");
        let producer =
            KubernetesProducer::new_seeded("default".to_string(), client, SourceBlock::none());
        let (tx, _rx) = mpsc::channel(8);

        let start = Instant::now();
        producer.start(tx);

        assert!(start.elapsed() < Duration::from_millis(50));
        producer.stop();
    }

    #[test]
    fn no_proxy_matches_exact_and_suffix() {
        assert!(super::host_bypasses_proxy("api.example.com", "example.com"));
        assert!(super::host_bypasses_proxy("example.com", "example.com"));
        assert!(super::host_bypasses_proxy(
            "k8s.internal",
            "other.com,.internal"
        ));
    }

    #[test]
    fn no_proxy_wildcard_and_empty() {
        assert!(super::host_bypasses_proxy("anything.local", "*"));
        assert!(!super::host_bypasses_proxy("api.example.com", ""));
        assert!(!super::host_bypasses_proxy("api.example.com", "  ,  "));
    }

    #[test]
    fn no_proxy_does_not_match_unrelated_or_partial_host() {
        assert!(!super::host_bypasses_proxy("api.example.com", "other.com"));
        // Suffix must align on a label boundary, not a substring.
        assert!(!super::host_bypasses_proxy("notexample.com", "example.com"));
    }
}
