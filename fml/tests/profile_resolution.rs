//! End-to-end profile + CLI override + `--skip-istio` resolution.
//!
//! Loads a TOML config from a temp dir, layers `--profile`, `--producer`,
//! and `--skip-istio` on top of it, and asserts the resulting compiled
//! producer list — both the spec ordering and the resolved `SourceBlock`
//! match semantics.

use std::fs;

use fml::{
    cli::{compile_resolved, resolve_producers},
    config::Config,
    log::Source,
    producer::ProducerSpec,
};

fn source(producer: &str, display: &str) -> Source {
    Source {
        producer: producer.to_string(),
        id: format!("{producer}/{display}"),
        display_name: display.to_string(),
        group: None,
    }
}

#[test]
fn profile_with_cli_override_and_skip_istio() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.toml"),
        r#"
        profile = "dev"

        [[profiles.dev.producers]]
        type = "demo"

        [[profiles.dev.producers]]
        type = "kubernetes"
        namespace = "team-a"
        blocked = "^debug-"

        [[profiles.dev.producers]]
        type = "kubernetes"
        namespace = "team-b"

        [[profiles.dev.producers]]
        type = "docker"
        blocked = "_postgres_"
        "#,
    )
    .unwrap();

    let config_dir = dir.path().to_str().unwrap();
    // We're not running the full Config::new() (which depends on $XDG_CONFIG_HOME);
    // load_with_config_dir is the public-equivalent unit-load entry point used
    // throughout the existing tests. Re-use it via the public Config::new path
    // by setting XDG_CONFIG_HOME, but since that's racy with serial_test we
    // construct directly through serde here.
    let raw = fs::read_to_string(dir.path().join("config.toml")).unwrap();
    let config: Config = toml::from_str(&raw).unwrap();

    // Override one kubernetes producer (replace team-a) and append a new demo.
    let cli = vec!["kubernetes:team-a".to_string(), "demo".to_string()];

    let pairs = resolve_producers(&config, None, &cli).unwrap();
    // Expected order: demo, kubernetes:team-a (replaced), kubernetes:team-b,
    // docker, demo (appended). The replace happens in place; the new demo
    // appends.
    assert_eq!(pairs.len(), 5);
    assert_eq!(pairs[0].0, ProducerSpec::Demo);
    assert_eq!(
        pairs[1].0,
        ProducerSpec::Kubernetes(Some("team-a".to_string()))
    );
    // CLI override drops the profile's `blocked`.
    assert!(pairs[1].1.blocked.is_none());
    assert!(!pairs[1].1.skip_istio);
    assert_eq!(
        pairs[2].0,
        ProducerSpec::Kubernetes(Some("team-b".to_string()))
    );
    assert_eq!(pairs[3].0, ProducerSpec::Docker);
    assert_eq!(pairs[4].0, ProducerSpec::Demo);

    let _ = config_dir;

    // Compile with --skip-istio.
    let resolved = compile_resolved(pairs, true);
    assert_eq!(resolved.len(), 5);

    // Demo (index 0): istio still allowed (no-op for demo).
    assert!(
        !resolved[0]
            .block
            .is_source_blocked(&source("demo", "istio-proxy"))
    );

    // Kubernetes team-a (overridden CLI): only istio applies; the original
    // `^debug-` regex was dropped.
    assert!(
        resolved[1]
            .block
            .is_source_blocked(&source("k8s", "istio-proxy-abc"))
    );
    assert!(
        !resolved[1]
            .block
            .is_source_blocked(&source("k8s", "debug-svc"))
    );

    // Kubernetes team-b: only istio applies.
    assert!(
        resolved[2]
            .block
            .is_source_blocked(&source("k8s", "productpage/istio-proxy"))
    );

    // Docker: profile regex `_postgres_` AND --skip-istio compose.
    assert!(
        resolved[3]
            .block
            .is_source_blocked(&source("docker", "team_postgres_1"))
    );
    assert!(
        resolved[3]
            .block
            .is_source_blocked(&source("docker", "istio-proxy"))
    );
    assert!(
        !resolved[3]
            .block
            .is_source_blocked(&source("docker", "api"))
    );
}

#[test]
fn invalid_regex_isolates_to_one_producer() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.toml"),
        r#"
        [[profiles.dev.producers]]
        type = "kubernetes"
        namespace = "good"
        blocked = "^valid"

        [[profiles.dev.producers]]
        type = "kubernetes"
        namespace = "bad"
        blocked = "(unclosed"

        [[profiles.dev.producers]]
        type = "docker"
        "#,
    )
    .unwrap();

    let raw = fs::read_to_string(dir.path().join("config.toml")).unwrap();
    let config: Config = toml::from_str(&raw).unwrap();

    let pairs = resolve_producers(&config, Some("dev"), &[]).unwrap();
    assert_eq!(pairs.len(), 3);

    // The bad-regex producer is dropped during compile; the others remain.
    let resolved = compile_resolved(pairs, false);
    assert_eq!(resolved.len(), 2);
    assert!(matches!(
        resolved[0].spec,
        ProducerSpec::Kubernetes(Some(ref ns)) if ns == "good"
    ));
    assert!(matches!(resolved[1].spec, ProducerSpec::Docker));
}
