//! CLI ↔ profile resolution.
//!
//! Combines the active profile's producer list with `--producer` CLI overrides
//! into a flat `Vec<(ProducerSpec, SourceBlockConfig)>`. `--producer` matches
//! profile entries by `(kind, disambiguator)`:
//!
//! - `demo` is repeatable; CLI `demo` always appends.
//! - `file:<path>` matches by exact path string equality.
//! - `docker` matches the single docker entry if present (multi is a config
//!   error, validated in [`ProfileConfig::validate`]).
//! - `kubernetes:<ns>` matches the kube entry with that namespace.
//!   `kubernetes` (bare) matches the unique kube entry if exactly one
//!   exists; with more than one, it is ambiguous and errors with the
//!   candidate list.
//!
//! A CLI override is treated as a brand-new producer config block: the
//! profile entry's `blocked` / `skip_istio` are dropped. `--skip-istio` is
//! the one composing flag and is applied later, after this resolution.
//!
//! `SourceBlock` compilation is intentionally out of scope here — this layer
//! returns raw [`SourceBlockConfig`] so per-producer regex compile failures
//! can be isolated by the caller.

use crate::{
    config::{Config, SourceBlockConfig},
    error::{FmlError, ProducerError},
    producer::{ProducerSpec, ResolvedProducer, SourceBlock},
};

/// Resolve the final ordered producer list from config, the active profile
/// name (CLI override winning over `config.profile`), and any `--producer`
/// CLI strings.
pub fn resolve_producers(
    config: &Config,
    profile_override: Option<&str>,
    cli_producers: &[String],
) -> Result<Vec<(ProducerSpec, SourceBlockConfig)>, FmlError> {
    let active_profile_name = profile_override.or(config.profile.as_deref());
    let profile = config.resolve_profile(active_profile_name)?;

    let mut working: Vec<(ProducerSpec, SourceBlockConfig)> = profile
        .map(|p| {
            p.producers
                .iter()
                .map(|c| (ProducerSpec::from(c), c.block_config()))
                .collect()
        })
        .unwrap_or_default();

    for raw in cli_producers {
        let spec = ProducerSpec::parse(raw)?;
        apply_cli_override(&mut working, spec)?;
    }

    Ok(working)
}

/// Compile each `(ProducerSpec, SourceBlockConfig)` into a [`ResolvedProducer`].
/// Per-entry compile failures are isolated: a bad regex on one producer
/// logs a warning and drops only that producer. Unrelated producers still
/// start. `force_skip_istio` is the global `--skip-istio` flag and is
/// applied to docker and kubernetes producers (only); `file`/`fake`/`demo`
/// ignore it.
pub fn compile_resolved(
    pairs: Vec<(ProducerSpec, SourceBlockConfig)>,
    force_skip_istio: bool,
) -> Vec<ResolvedProducer> {
    let mut out = Vec::with_capacity(pairs.len());
    for (spec, cfg) in pairs {
        let force =
            force_skip_istio && matches!(spec, ProducerSpec::Docker | ProducerSpec::Kubernetes(_));
        match SourceBlock::from_config(&cfg, force) {
            Ok(block) => out.push(ResolvedProducer { spec, block }),
            Err(err) => {
                tracing::warn!(
                    "skipping producer {:?}: invalid `blocked` regex: {err}",
                    spec
                );
            }
        }
    }
    out
}

fn apply_cli_override(
    working: &mut Vec<(ProducerSpec, SourceBlockConfig)>,
    cli_spec: ProducerSpec,
) -> Result<(), FmlError> {
    match &cli_spec {
        ProducerSpec::Demo => {
            working.push((cli_spec, SourceBlockConfig::default()));
            Ok(())
        }
        ProducerSpec::File(path) => {
            let idx = working.iter().position(|(s, _)| match s {
                ProducerSpec::File(p) => p == path,
                _ => false,
            });
            match idx {
                Some(i) => working[i] = (cli_spec, SourceBlockConfig::default()),
                None => working.push((cli_spec, SourceBlockConfig::default())),
            }
            Ok(())
        }
        ProducerSpec::Docker => {
            let idx = working
                .iter()
                .position(|(s, _)| matches!(s, ProducerSpec::Docker));
            match idx {
                Some(i) => working[i] = (cli_spec, SourceBlockConfig::default()),
                None => working.push((cli_spec, SourceBlockConfig::default())),
            }
            Ok(())
        }
        ProducerSpec::Kubernetes(ns) => match ns {
            Some(target_ns) => {
                let idx = working.iter().position(|(s, _)| match s {
                    ProducerSpec::Kubernetes(Some(n)) => n == target_ns,
                    _ => false,
                });
                match idx {
                    Some(i) => working[i] = (cli_spec, SourceBlockConfig::default()),
                    None => working.push((cli_spec, SourceBlockConfig::default())),
                }
                Ok(())
            }
            None => {
                let kube_indices: Vec<usize> = working
                    .iter()
                    .enumerate()
                    .filter_map(|(i, (s, _))| matches!(s, ProducerSpec::Kubernetes(_)).then_some(i))
                    .collect();
                match kube_indices.as_slice() {
                    [] => {
                        working.push((cli_spec, SourceBlockConfig::default()));
                        Ok(())
                    }
                    [i] => {
                        working[*i] = (cli_spec, SourceBlockConfig::default());
                        Ok(())
                    }
                    _ => {
                        let candidates: Vec<String> = kube_indices
                            .iter()
                            .map(|i| match &working[*i].0 {
                                ProducerSpec::Kubernetes(Some(n)) => format!("kubernetes:{n}"),
                                ProducerSpec::Kubernetes(None) => "kubernetes".to_string(),
                                _ => unreachable!(),
                            })
                            .collect();
                        Err(FmlError::Producer(ProducerError::Cli(format!(
                            "`--producer kubernetes` is ambiguous; the active profile contains multiple kubernetes producers: {}. Re-run with --producer kubernetes:<namespace>.",
                            candidates.join(", ")
                        ))))
                    }
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::{ProducerConfig, ProfileConfig};

    fn config_with_profile(name: &str, producers: Vec<ProducerConfig>) -> Config {
        let mut config = Config {
            profile: Some(name.to_string()),
            ..Config::default()
        };
        config.profiles.insert(
            name.to_string(),
            ProfileConfig {
                theme: None,
                producers,
            },
        );
        config
    }

    #[test]
    fn profile_only_resolution() {
        let config = config_with_profile(
            "dev",
            vec![
                ProducerConfig::Demo,
                ProducerConfig::Kubernetes {
                    namespace: Some("a".to_string()),
                    block: SourceBlockConfig {
                        blocked: Some("^x".to_string()),
                        skip_istio: false,
                    },
                },
            ],
        );

        let resolved = resolve_producers(&config, None, &[]).unwrap();
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].0, ProducerSpec::Demo);
        assert_eq!(
            resolved[1].0,
            ProducerSpec::Kubernetes(Some("a".to_string()))
        );
        assert_eq!(resolved[1].1.blocked.as_deref(), Some("^x"));
    }

    #[test]
    fn cli_only_resolution() {
        let config = Config::default();
        let resolved =
            resolve_producers(&config, None, &["demo".to_string(), "docker".to_string()]).unwrap();
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].0, ProducerSpec::Demo);
        assert_eq!(resolved[1].0, ProducerSpec::Docker);
    }

    #[test]
    fn cli_kube_namespace_replaces_matching_profile_entry() {
        let config = config_with_profile(
            "dev",
            vec![
                ProducerConfig::Kubernetes {
                    namespace: Some("a".to_string()),
                    block: SourceBlockConfig {
                        blocked: Some("^x".to_string()),
                        skip_istio: true,
                    },
                },
                ProducerConfig::Kubernetes {
                    namespace: Some("b".to_string()),
                    block: SourceBlockConfig::default(),
                },
            ],
        );

        let resolved = resolve_producers(&config, None, &["kubernetes:a".to_string()]).unwrap();
        assert_eq!(resolved.len(), 2);
        assert_eq!(
            resolved[0].0,
            ProducerSpec::Kubernetes(Some("a".to_string()))
        );
        // CLI override drops the profile's block config.
        assert_eq!(resolved[0].1, SourceBlockConfig::default());
    }

    #[test]
    fn cli_appends_when_no_match() {
        let config = config_with_profile(
            "dev",
            vec![ProducerConfig::Kubernetes {
                namespace: Some("a".to_string()),
                block: SourceBlockConfig::default(),
            }],
        );

        let resolved = resolve_producers(&config, None, &["kubernetes:b".to_string()]).unwrap();
        assert_eq!(resolved.len(), 2);
        assert_eq!(
            resolved[1].0,
            ProducerSpec::Kubernetes(Some("b".to_string()))
        );
    }

    #[test]
    fn ambiguous_bare_kubernetes_override_errors() {
        let config = config_with_profile(
            "dev",
            vec![
                ProducerConfig::Kubernetes {
                    namespace: Some("a".to_string()),
                    block: SourceBlockConfig::default(),
                },
                ProducerConfig::Kubernetes {
                    namespace: Some("b".to_string()),
                    block: SourceBlockConfig::default(),
                },
            ],
        );

        let err = resolve_producers(&config, None, &["kubernetes".to_string()]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("ambiguous"));
        assert!(msg.contains("kubernetes:a"));
        assert!(msg.contains("kubernetes:b"));
    }

    #[test]
    fn cli_demo_appends_rather_than_overrides() {
        let config = config_with_profile("dev", vec![ProducerConfig::Demo]);
        let resolved = resolve_producers(&config, None, &["demo".to_string()]).unwrap();
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].0, ProducerSpec::Demo);
        assert_eq!(resolved[1].0, ProducerSpec::Demo);
    }

    #[test]
    fn missing_profile_aborts_with_available_list() {
        let config = config_with_profile("dev", vec![ProducerConfig::Demo]);
        let err = resolve_producers(&config, Some("missing"), &[]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("missing"));
        assert!(msg.contains("dev"));
    }

    #[test]
    fn cli_profile_overrides_config_profile() {
        let mut config = config_with_profile("dev", vec![ProducerConfig::Demo]);
        config.profiles.insert(
            "prod".to_string(),
            ProfileConfig {
                theme: None,
                producers: vec![ProducerConfig::Docker {
                    block: SourceBlockConfig::default(),
                }],
            },
        );

        let resolved = resolve_producers(&config, Some("prod"), &[]).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].0, ProducerSpec::Docker);
    }

    #[test]
    fn multi_docker_in_profile_fails_validation() {
        let config = config_with_profile(
            "dev",
            vec![
                ProducerConfig::Docker {
                    block: SourceBlockConfig::default(),
                },
                ProducerConfig::Docker {
                    block: SourceBlockConfig::default(),
                },
            ],
        );
        assert!(resolve_producers(&config, None, &[]).is_err());
    }

    #[test]
    fn cli_override_drops_profile_block_config() {
        let config = config_with_profile(
            "dev",
            vec![ProducerConfig::Docker {
                block: SourceBlockConfig {
                    blocked: Some("_postgres_".to_string()),
                    skip_istio: true,
                },
            }],
        );

        let resolved = resolve_producers(&config, None, &["docker".to_string()]).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].1, SourceBlockConfig::default());
    }

    fn fake_source(producer: &str, display: &str) -> crate::log::Source {
        crate::log::Source {
            producer: producer.to_string(),
            id: format!("{producer}/{display}"),
            display_name: display.to_string(),
            group: None,
        }
    }

    #[test]
    fn skip_istio_flag_blocks_istio_for_kube_and_docker() {
        let pairs = vec![
            (ProducerSpec::Docker, SourceBlockConfig::default()),
            (
                ProducerSpec::Kubernetes(Some("a".to_string())),
                SourceBlockConfig::default(),
            ),
            (ProducerSpec::Demo, SourceBlockConfig::default()),
        ];
        let resolved = compile_resolved(pairs, true);
        assert_eq!(resolved.len(), 3);
        // Docker + Kubernetes block istio-proxy.
        assert!(
            resolved[0]
                .block
                .is_source_blocked(&fake_source("docker", "istio-proxy"))
        );
        assert!(
            resolved[1]
                .block
                .is_source_blocked(&fake_source("kubernetes", "istio-proxy"))
        );
        // Demo does not have istio applied (no-op for demo).
        assert!(
            !resolved[2]
                .block
                .is_source_blocked(&fake_source("demo", "istio-proxy"))
        );
    }

    #[test]
    fn skip_istio_flag_composes_with_existing_block_regex() {
        let pairs = vec![(
            ProducerSpec::Docker,
            SourceBlockConfig {
                blocked: Some("_postgres_".to_string()),
                skip_istio: false,
            },
        )];
        let resolved = compile_resolved(pairs, true);
        assert_eq!(resolved.len(), 1);
        // Original regex still applies.
        assert!(
            resolved[0]
                .block
                .is_source_blocked(&fake_source("docker", "team_postgres_1"))
        );
        // Plus the istio shortcut.
        assert!(
            resolved[0]
                .block
                .is_source_blocked(&fake_source("docker", "istio-proxy-abc"))
        );
    }

    #[test]
    fn skip_istio_flag_is_noop_for_file_and_demo() {
        let pairs = vec![
            (
                ProducerSpec::File(std::path::PathBuf::from("/tmp/log")),
                SourceBlockConfig::default(),
            ),
            (ProducerSpec::Demo, SourceBlockConfig::default()),
        ];
        let resolved = compile_resolved(pairs, true);
        assert_eq!(resolved.len(), 2);
        for r in &resolved {
            assert!(
                !r.block.is_source_blocked(&fake_source("p", "istio-proxy")),
                "skip_istio must be a no-op for file/demo"
            );
        }
    }

    #[test]
    fn invalid_regex_is_isolated_to_one_producer() {
        let pairs = vec![
            (ProducerSpec::Demo, SourceBlockConfig::default()),
            (
                ProducerSpec::Kubernetes(Some("a".to_string())),
                SourceBlockConfig {
                    blocked: Some("(unclosed".to_string()),
                    skip_istio: false,
                },
            ),
            (ProducerSpec::Docker, SourceBlockConfig::default()),
        ];
        let resolved = compile_resolved(pairs, false);
        // The kubernetes producer is dropped; the other two remain.
        assert_eq!(resolved.len(), 2);
        assert!(matches!(resolved[0].spec, ProducerSpec::Demo));
        assert!(matches!(resolved[1].spec, ProducerSpec::Docker));
    }

    #[test]
    fn cli_file_matches_exact_path_string() {
        let config = config_with_profile(
            "dev",
            vec![ProducerConfig::File {
                file: PathBuf::from("/var/log/a.log"),
            }],
        );

        let resolved =
            resolve_producers(&config, None, &["file:/var/log/a.log".to_string()]).unwrap();
        assert_eq!(resolved.len(), 1);

        let resolved =
            resolve_producers(&config, None, &["file:/var/log/b.log".to_string()]).unwrap();
        assert_eq!(resolved.len(), 2);
    }
}
