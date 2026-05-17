//! Per-producer config types.
//!
//! Each `ProducerConfig` is the typed TOML form of a producer
//! (`type = "demo" | "file" | "docker" | "kubernetes"`) and carries a
//! flattened [`SourceBlockConfig`] for the docker and kubernetes kinds
//! (`blocked = "<regex>"`, `skip_istio = true`).
//!
//! Conversion to the runtime [`ProducerSpec`] strips the block config: the
//! spec is reused by the CLI parser which has no block info, while the block
//! config is compiled separately into a `SourceBlock` matcher. Callers pair
//! the two explicitly, e.g. `Vec<(ProducerSpec, SourceBlockConfig)>`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::producer::ProducerSpec;

/// Per-producer source-blocking configuration.
///
/// `blocked` is a regex matched against `source.id` OR `source.display_name`.
/// `skip_istio` is a shortcut adding the literal `"istio-proxy"` substring
/// to the matcher. Both compose; they never override each other.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SourceBlockConfig {
    #[serde(default)]
    pub blocked: Option<String>,
    #[serde(default)]
    pub skip_istio: bool,
}

/// Typed TOML form of a single producer entry inside a profile.
///
/// `type` discriminates the variant (`"demo" | "file" | "docker" | "kubernetes"`).
/// Docker and kubernetes entries accept the [`SourceBlockConfig`] keys
/// (`blocked`, `skip_istio`) flattened at the entry level.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ProducerConfig {
    Demo,
    File {
        file: PathBuf,
    },
    Docker {
        #[serde(flatten, default)]
        block: SourceBlockConfig,
    },
    Kubernetes {
        #[serde(default)]
        namespace: Option<String>,
        #[serde(flatten, default)]
        block: SourceBlockConfig,
    },
}

impl ProducerConfig {
    /// The block config attached to this producer entry, if any. `Demo` and
    /// `File` always return the default (no-op) block config.
    pub fn block_config(&self) -> SourceBlockConfig {
        match self {
            ProducerConfig::Demo | ProducerConfig::File { .. } => SourceBlockConfig::default(),
            ProducerConfig::Docker { block } => block.clone(),
            ProducerConfig::Kubernetes { block, .. } => block.clone(),
        }
    }
}

impl From<&ProducerConfig> for ProducerSpec {
    fn from(cfg: &ProducerConfig) -> Self {
        match cfg {
            ProducerConfig::Demo => ProducerSpec::Demo,
            ProducerConfig::File { file } => ProducerSpec::File(file.clone()),
            ProducerConfig::Docker { .. } => ProducerSpec::Docker,
            ProducerConfig::Kubernetes { namespace, .. } => {
                ProducerSpec::Kubernetes(namespace.clone())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn producer_config_demo_round_trips() {
        let toml_str = r#"type = "demo""#;
        let cfg: ProducerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg, ProducerConfig::Demo);
        assert_eq!(ProducerSpec::from(&cfg), ProducerSpec::Demo);
    }

    #[test]
    fn producer_config_file_round_trips() {
        let toml_str = r#"
            type = "file"
            file = "/var/log/app.log"
        "#;
        let cfg: ProducerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            cfg,
            ProducerConfig::File {
                file: PathBuf::from("/var/log/app.log"),
            }
        );
        assert_eq!(
            ProducerSpec::from(&cfg),
            ProducerSpec::File(PathBuf::from("/var/log/app.log"))
        );
    }

    #[test]
    fn producer_config_docker_round_trips_with_block() {
        let toml_str = r#"
            type = "docker"
            blocked = "_test_"
            skip_istio = true
        "#;
        let cfg: ProducerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(ProducerSpec::from(&cfg), ProducerSpec::Docker);
        let block = cfg.block_config();
        assert_eq!(block.blocked.as_deref(), Some("_test_"));
        assert!(block.skip_istio);
    }

    #[test]
    fn producer_config_kubernetes_round_trips_with_namespace() {
        let toml_str = r#"
            type = "kubernetes"
            namespace = "prod"
            blocked = "^istio"
        "#;
        let cfg: ProducerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            ProducerSpec::from(&cfg),
            ProducerSpec::Kubernetes(Some("prod".to_string()))
        );
        assert_eq!(cfg.block_config().blocked.as_deref(), Some("^istio"));
    }

    #[test]
    fn producer_config_kubernetes_default_block_is_empty() {
        let toml_str = r#"
            type = "kubernetes"
            namespace = "prod"
        "#;
        let cfg: ProducerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.block_config(), SourceBlockConfig::default());
    }
}
