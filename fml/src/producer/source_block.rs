//! Source-level block matcher.
//!
//! A [`SourceBlock`] is a compiled matcher used by producers to decide
//! whether a discovered [`Source`] should be silently dropped before any
//! `SourceFound` or `StoreEvent` is emitted for it.
//!
//! ## Match semantics
//!
//! Both the configured `blocked` regex and the literal substrings (added
//! by `skip_istio = true` or the global `--skip-istio` flag) are tested
//! against `source.id` AND `source.display_name`. A match against either
//! field blocks the source.
//!
//! Substring matches use plain `str::contains` (case-sensitive), which
//! handles the realistic identifier shapes — `istio-proxy-abc123`,
//! `productpage/istio-proxy`, etc. — without requiring users to write a
//! regex.
//!
//! ## Lifecycle and contract
//!
//! Blocks are static for the process lifetime: the matcher is compiled
//! once during startup and then read-only. There is no runtime mutation
//! API and no UI affordance to unblock; users restart with a different
//! profile or CLI flags. Producers are responsible for never emitting
//! events (`SourceFound`, `SourceLost`, `StoreEvent`) for a blocked
//! source — see the producer-side audit comments in
//! `producer/kubernetes.rs` and `producer/docker.rs`.
//!
//! ## Future
//!
//! A sibling `LineBlock` (for per-entry redaction/filtering) is intended
//! to live alongside this type rather than replace or generalise it.
//! Keep `SourceBlock`'s API focused on source-level matching.

use regex::Regex;

use crate::{config::SourceBlockConfig, log::Source, producer::ProducerSpec};

const ISTIO_PROXY_NEEDLE: &str = "istio-proxy";

/// Compiled source-level block matcher. See the [module docs](self) for
/// match semantics, lifecycle, and contract.
#[derive(Debug, Clone, Default)]
pub struct SourceBlock {
    regex: Option<Regex>,
    substrings: Vec<String>,
}

impl SourceBlock {
    /// A no-op matcher that blocks nothing. Used for producers that do not
    /// support blocking (e.g. `file`, `fake`) so the producer interface
    /// stays uniform.
    pub fn none() -> Self {
        Self::default()
    }

    /// Compile a `SourceBlockConfig` into a matcher.
    ///
    /// `force_skip_istio` ORs `skip_istio = true` in regardless of the
    /// config — used by the global `--skip-istio` CLI flag, which always
    /// strengthens (never weakens) the matcher.
    ///
    /// Returns `regex::Error` if the configured `blocked` regex is invalid;
    /// callers (`compile_resolved` in `cli.rs`) per-producer-isolate this
    /// failure rather than aborting startup.
    pub fn from_config(
        cfg: &SourceBlockConfig,
        force_skip_istio: bool,
    ) -> Result<Self, regex::Error> {
        let regex = cfg
            .blocked
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(Regex::new)
            .transpose()?;

        let mut substrings = Vec::new();
        if cfg.skip_istio || force_skip_istio {
            substrings.push(ISTIO_PROXY_NEEDLE.to_string());
        }

        Ok(Self { regex, substrings })
    }

    /// Test whether `source` should be blocked. Producers must call this
    /// before every `SourceFound` / `StoreEvent` they emit for the source.
    pub fn is_source_blocked(&self, source: &Source) -> bool {
        for needle in &self.substrings {
            if source.id.contains(needle) || source.display_name.contains(needle) {
                return true;
            }
        }
        if let Some(r) = &self.regex {
            return r.is_match(&source.id) || r.is_match(&source.display_name);
        }
        false
    }
}

/// A producer specification paired with its compiled block matcher. This
/// is the form `App::new` consumes; resolution (`resolve_producers`)
/// produces the un-compiled pair, then `compile_resolved` turns it into
/// `Vec<ResolvedProducer>` with per-entry failure isolation.
#[derive(Debug, Clone)]
pub struct ResolvedProducer {
    pub spec: ProducerSpec,
    pub block: SourceBlock,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::Source;

    fn source(id: &str, display: &str) -> Source {
        Source {
            producer: "test".to_string(),
            id: id.to_string(),
            display_name: display.to_string(),
            group: None,
        }
    }

    #[test]
    fn empty_matcher_blocks_nothing() {
        let block = SourceBlock::none();
        assert!(!block.is_source_blocked(&source("a", "A")));
    }

    #[test]
    fn regex_matches_id() {
        let cfg = SourceBlockConfig {
            blocked: Some("^pg-".to_string()),
            skip_istio: false,
        };
        let block = SourceBlock::from_config(&cfg, false).unwrap();
        assert!(block.is_source_blocked(&source("pg-1", "postgres-1")));
        assert!(!block.is_source_blocked(&source("api", "api")));
    }

    #[test]
    fn regex_matches_display_name() {
        let cfg = SourceBlockConfig {
            blocked: Some("postgres".to_string()),
            skip_istio: false,
        };
        let block = SourceBlock::from_config(&cfg, false).unwrap();
        assert!(block.is_source_blocked(&source("svc-99", "team/postgres")));
    }

    #[test]
    fn skip_istio_substring_match_with_suffix() {
        let cfg = SourceBlockConfig {
            blocked: None,
            skip_istio: true,
        };
        let block = SourceBlock::from_config(&cfg, false).unwrap();
        assert!(block.is_source_blocked(&source("istio-proxy-abc123", "istio-proxy-abc123")));
    }

    #[test]
    fn skip_istio_substring_match_with_prefix() {
        let cfg = SourceBlockConfig {
            blocked: None,
            skip_istio: true,
        };
        let block = SourceBlock::from_config(&cfg, false).unwrap();
        assert!(block.is_source_blocked(&source("pod-1", "productpage/istio-proxy")));
    }

    #[test]
    fn force_skip_istio_overrides_config_false() {
        let cfg = SourceBlockConfig::default();
        let block = SourceBlock::from_config(&cfg, true).unwrap();
        assert!(block.is_source_blocked(&source("istio-proxy-x", "x")));
    }

    #[test]
    fn skip_istio_does_not_match_unrelated_source() {
        let cfg = SourceBlockConfig {
            blocked: None,
            skip_istio: true,
        };
        let block = SourceBlock::from_config(&cfg, false).unwrap();
        assert!(!block.is_source_blocked(&source("api-1", "api-1")));
    }

    #[test]
    fn combined_regex_and_skip_istio_both_apply() {
        let cfg = SourceBlockConfig {
            blocked: Some("postgres".to_string()),
            skip_istio: true,
        };
        let block = SourceBlock::from_config(&cfg, false).unwrap();
        assert!(block.is_source_blocked(&source("pg", "team/postgres")));
        assert!(block.is_source_blocked(&source("istio-proxy-1", "x")));
        assert!(!block.is_source_blocked(&source("api", "api")));
    }

    #[test]
    fn invalid_regex_returns_error() {
        let cfg = SourceBlockConfig {
            blocked: Some("(unclosed".to_string()),
            skip_istio: false,
        };
        assert!(SourceBlock::from_config(&cfg, false).is_err());
    }

    #[test]
    fn empty_blocked_string_is_treated_as_none() {
        let cfg = SourceBlockConfig {
            blocked: Some("".to_string()),
            skip_istio: false,
        };
        let block = SourceBlock::from_config(&cfg, false).unwrap();
        assert!(!block.is_source_blocked(&source("a", "A")));
    }
}
