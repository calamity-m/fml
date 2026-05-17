//! Profile config type.
//!
//! `ProfileConfig` is the TOML shape for `[profiles.<name>]`; a profile owns
//! a list of [`ProducerConfig`] entries and an optional TUI theme override.

use serde::{Deserialize, Serialize};

use crate::{config::producer::ProducerConfig, error::FmlError};

/// A named startup bundle of producers.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProfileConfig {
    /// Optional TUI theme override applied when this profile is active.
    #[serde(default)]
    pub theme: Option<String>,

    #[serde(default)]
    pub producers: Vec<ProducerConfig>,
}

impl ProfileConfig {
    /// Apply structural rules that cannot be expressed via serde alone:
    /// at most one `docker` producer per profile (a second is a config
    /// error). `demo` is repeatable; `kubernetes` may appear multiple times
    /// with different namespaces; `file` may appear multiple times with
    /// different paths.
    pub fn validate(&self, name: &str) -> Result<(), FmlError> {
        let docker_count = self
            .producers
            .iter()
            .filter(|p| matches!(p, ProducerConfig::Docker { .. }))
            .count();
        if docker_count > 1 {
            return Err(FmlError::Profile(format!(
                "profile `{name}` has {docker_count} docker producers; at most one is allowed"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::producer::SourceBlockConfig;

    #[test]
    fn profile_validate_allows_multiple_demo_entries() {
        let profile = ProfileConfig {
            theme: None,
            producers: vec![ProducerConfig::Demo, ProducerConfig::Demo],
        };
        profile.validate("p").unwrap();
    }

    #[test]
    fn profile_validate_rejects_multiple_docker_entries() {
        let profile = ProfileConfig {
            theme: None,
            producers: vec![
                ProducerConfig::Docker {
                    block: SourceBlockConfig::default(),
                },
                ProducerConfig::Docker {
                    block: SourceBlockConfig::default(),
                },
            ],
        };
        let err = profile.validate("p").unwrap_err();
        assert!(err.to_string().contains("docker"));
    }

    #[test]
    fn profile_validate_allows_multiple_kube_namespaces() {
        let profile = ProfileConfig {
            theme: None,
            producers: vec![
                ProducerConfig::Kubernetes {
                    namespace: Some("a".to_string()),
                    block: SourceBlockConfig::default(),
                },
                ProducerConfig::Kubernetes {
                    namespace: Some("b".to_string()),
                    block: SourceBlockConfig::default(),
                },
            ],
        };
        profile.validate("p").unwrap();
    }
}
