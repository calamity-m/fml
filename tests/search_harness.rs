#![allow(unused)]
//! Search layer integration harness.
//!
//! # What this covers
//!
//! This is the **most critical harness** in the test suite. The greedy
//! expansion algorithm is the core differentiator of fml, and subtle bugs in
//! the semantic graph traversal are hard to catch by inspection alone.
//!
//! - **Greed monotonicity**: for any query `q`, the result set at greed level G
//!   must be a superset of the result set at greed level G-1. This invariant
//!   must hold across all greed levels 0–10 and for every query term in the
//!   ontology. Violations mean the graph traversal is not correctly expanding.
//! - **Domain families**: each of the 7 domain families (auth, error, network,
//!   database, performance, lifecycle, resource) has at least one test verifying
//!   that related terms surface at an appropriate greed level.
//! - **Negative prefix inference**: queries starting with negative prefixes
//!   (`un-`, `fail-`, `err-`, `invalid-`, `no-`) must bias toward the error
//!   and failure clusters without explicitly encoding every negative form.
//! - **Key-value filter**: `level:error` must restrict results to entries with
//!   `entry.level == Error` before term expansion runs.
//! - **Exact mode**: greed = 0 bypasses expansion and does literal substring
//!   or regex match.
//! - **Property: results ⊆ store**: no result may appear that is not in the
//!   store. Search must never fabricate entries.
//! - **Property: greed monotonicity** (proptest variant): for random queries
//!   and random corpora, greed G results ⊇ greed G-1 results.
//!
//! # What this does NOT cover
//!
//! - UI highlighting of matched spans (see ui_harness)
//! - Persistence of search state across sessions
//!
//! # Running
//!
//! ```sh
//! cargo test --test search_harness
//! cargo test --test search_harness -- --nocapture
//! ```

mod common;
use common::*;
use fml_core::LogLevel;

// ---------------------------------------------------------------------------
// Exact mode (greed = 0)
// ---------------------------------------------------------------------------

/// Exact mode does a literal substring match — "timeout" matches lines
/// containing the exact string "timeout", nothing else.
#[test]
#[ignore = "not yet implemented"]
fn exact_mode_literal_substring() {
    todo!("build store with timeout/connection lines; greed=0 query 'timeout'; assert only literal matches")
}

/// Exact mode regex: `/time.*out/` matches timeout, timed_out, time_out, etc.
#[test]
#[ignore = "not yet implemented"]
fn exact_mode_regex_match() {
    todo!("query with regex pattern; assert regex-matched lines returned")
}

/// Exact mode does NOT return synonyms (e.g. "timeout" should not return "latency").
#[test]
#[ignore = "not yet implemented"]
fn exact_mode_does_not_expand() {
    todo!("build store with 'latency' lines; greed=0 query 'timeout'; assert no results")
}

// ---------------------------------------------------------------------------
// Greed monotonicity
// ---------------------------------------------------------------------------

/// **Critical invariant.** For the query "auth", results at greed=3 must be a
/// superset of results at greed=1.
#[test]
#[ignore = "not yet implemented"]
fn greed_3_superset_of_greed_1_auth() {
    todo!("build auth corpus; assert results(greed=3) ⊇ results(greed=1)")
}

/// For the query "error", results at greed=7 must be a superset of results at
/// greed=3.
#[test]
#[ignore = "not yet implemented"]
fn greed_7_superset_of_greed_3_error() {
    todo!("build error corpus; assert results(greed=7) ⊇ results(greed=3)")
}

/// For the query "timeout", results at greed=10 must be a superset of results
/// at greed=7.
#[test]
#[ignore = "not yet implemented"]
fn greed_10_superset_of_greed_7_timeout() {
    todo!("build network corpus; assert results(greed=10) ⊇ results(greed=7)")
}

/// Greed monotonicity holds for all levels 0–10 for the "auth" query.
#[test]
#[ignore = "not yet implemented"]
fn greed_monotonicity_all_levels_auth() {
    todo!("for each consecutive pair (g, g+1) in 0..=10; assert results(g+1) ⊇ results(g)")
}

// ---------------------------------------------------------------------------
// Domain families
// ---------------------------------------------------------------------------

/// **auth family**: at greed >= 3, the query "auth" surfaces "login",
/// "credential", and "session" terms.
#[test]
#[ignore = "not yet implemented"]
fn auth_domain_surfaces_synonyms_at_greed_3() {
    todo!("build store with login/credential/session lines; greed=3 'auth'; assert all match")
}

/// **auth family**: at greed >= 7, "auth" surfaces "token", "permission",
/// "principal".
#[test]
#[ignore = "not yet implemented"]
fn auth_domain_surfaces_peers_at_greed_7() {
    todo!("build store with token/permission lines; greed=7 'auth'; assert they match")
}

/// **error family**: "error" at greed >= 3 surfaces "exception", "failure",
/// "panic".
#[test]
#[ignore = "not yet implemented"]
fn error_domain_surfaces_synonyms_at_greed_3() {
    todo!("build store with exception/failure/panic lines; greed=3 'error'; assert all match")
}

/// **network family**: "timeout" at greed >= 3 surfaces "connection", "retry",
/// "refused".
#[test]
#[ignore = "not yet implemented"]
fn network_domain_surfaces_synonyms_at_greed_3() {
    todo!("build network corpus; greed=3 'timeout'; assert connection/retry/refused match")
}

/// **database family**: "query" at greed >= 3 surfaces "deadlock",
/// "transaction", "rollback".
#[test]
#[ignore = "not yet implemented"]
fn database_domain_surfaces_synonyms() {
    todo!("build db corpus; greed=3 'query'; assert deadlock/transaction/rollback match")
}

/// **performance family**: "slow" at greed >= 3 surfaces "latency", "elapsed",
/// "duration".
#[test]
#[ignore = "not yet implemented"]
fn performance_domain_surfaces_synonyms() {
    todo!("build perf corpus; greed=3 'slow'; assert latency/elapsed/duration match")
}

/// **lifecycle family**: "startup" at greed >= 3 surfaces "init", "ready",
/// "healthy".
#[test]
#[ignore = "not yet implemented"]
fn lifecycle_domain_surfaces_synonyms() {
    todo!("build lifecycle corpus; greed=3 'startup'; assert init/ready/healthy match")
}

/// **resource family**: "oom" at greed >= 3 surfaces "memory", "limit",
/// "exhausted".
#[test]
#[ignore = "not yet implemented"]
fn resource_domain_surfaces_synonyms() {
    todo!("build resource corpus; greed=3 'oom'; assert memory/limit/exhausted match")
}

// ---------------------------------------------------------------------------
// Negative prefix inference
// ---------------------------------------------------------------------------

/// "unauth" at greed >= 7 surfaces "forbidden", "rejected", "denied",
/// and "401" — terms never explicitly linked to "unauth" in the ontology.
#[test]
#[ignore = "not yet implemented"]
fn unauth_at_high_greed_surfaces_denial_terms() {
    todo!(
        "build store with forbidden/rejected/denied/401 lines; greed=7 'unauth'; assert all match"
    )
}

/// "fail" prefix biases toward the error cluster — "failure", "failed",
/// "exception" surface at greed >= 3.
#[test]
#[ignore = "not yet implemented"]
fn fail_prefix_biases_toward_error_cluster() {
    todo!("greed=3 'fail'; assert error cluster terms surface")
}

/// "invalid" prefix biases toward auth error terms.
#[test]
#[ignore = "not yet implemented"]
fn invalid_prefix_biases_toward_auth_errors() {
    todo!("greed=3 'invalid'; assert invalid_token / unauthorized surface")
}

// ---------------------------------------------------------------------------
// Key-value filter
// ---------------------------------------------------------------------------

/// `level:error` restricts results to ERROR entries before expansion runs.
#[test]
#[ignore = "not yet implemented"]
fn kv_filter_level_restricts_results() {
    todo!("build mixed-level store; query 'level:error'; assert only Error/Fatal entries returned")
}

/// `level:error timeout` at greed=3 expands 'timeout' but still filters to
/// ERROR entries only.
#[test]
#[ignore = "not yet implemented"]
fn kv_filter_combined_with_term_expansion() {
    todo!("build store; query 'level:error timeout' greed=3; assert expansion AND level filter both apply")
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

/// Property: search results are always a subset of the store contents.
/// No entry returned by search may be absent from the store.
#[test]
#[ignore = "not yet implemented"]
fn prop_results_subset_of_store() {
    todo!("proptest: random corpus and query; assert every result entry exists in the store")
}

/// Property: greed monotonicity holds for random queries and random corpora.
/// For all consecutive greed pairs (g, g+1): results(g+1) ⊇ results(g).
#[test]
#[ignore = "not yet implemented"]
fn prop_greed_monotonicity() {
    todo!("proptest: random corpus and term; for g in 0..10 assert results(g+1) ⊇ results(g)")
}

/// Property: backwards resolution — for every (A, B) pair where A expands to
/// include B at greed G, B must expand to include A at some greed H where
/// G < H ≤ 10. Verified across all ontology pairs.
#[test]
#[ignore = "not yet implemented"]
fn prop_backwards_resolution() {
    todo!("for every (seed, peer) pair in the ontology: assert that peer expands to include seed at some greed level higher than the forward direction")
}
