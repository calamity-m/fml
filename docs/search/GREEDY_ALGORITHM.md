# Greedy Search Algorithm

The search expansion system works in layers, each activated by a **greed level** (0–10). At low greed, only close morphological matches are returned. As greed increases, the search walks further across a semantic graph to find domain-related terms the user didn't explicitly type.

## Layers

| Greed | Layer | Example (`auth`) |
|-------|-------|-----------------|
| 0 | Exact only (no expansion) | `auth` |
| 1–2 | Morphological / prefix | `authenticated`, `authorization` |
| 3–4 | Synonym / ontology cluster | `login`, `credential`, `session` |
| 5–6 | Domain peers (1 hop) | `token`, `permission`, `principal` |
| 7–8 | Domain peers (2 hops) | `bearer`, `jwt`, `oauth` |
| 9–10 | Domain peers (3+ hops) | `expiry`, `role`, `identity` |

Multi-hop traversal is how distant-but-related terms are reached. `auth → token → bearer` is two hops; at low greed that path is never walked.

## Semantic graph structure

The graph is a **directed weighted graph** where edges carry both a relationship type and a weight. The greed slider controls the minimum edge weight traversed and the maximum traversal depth.

```rust
struct TermNode {
    term: String,
    relations: Vec<(RelationType, String, f32)>, // (type, target, weight)
}

enum RelationType {
    Morphological, // auth -> authenticated
    Synonym,       // error -> failure
    DomainPeer,    // auth -> token
    Hypernym,      // unauthorized -> auth
    Implication,   // panic -> crash
}
```

Edges are bidirectional but **not symmetric in weight** — `auth → token` may be 0.8 while `token → auth` is only 0.5, reflecting that "token" is a broader search context than "auth".

## Greed thresholds

| Greed | min_weight | max_depth |
|-------|-----------|-----------|
| 0 | exact | 0 |
| 1–2 | 0.95 | 1 (morphological only) |
| 3–4 | 0.75 | 1 |
| 5–6 | 0.55 | 1 |
| 7–8 | 0.40 | 2 |
| 9–10 | 0.25 | 3+ |

## Ontology definition

Clusters are defined in a static TOML file (compiled in via `include_str!` or `phf`). Approximately 150–200 terms across ~10 domain families covers the vast majority of real-world log patterns.

```toml
[[cluster]]
seed = "auth"
morphological = ["authenticate", "authenticated", "authentication", "authorized", "authorization"]
synonyms      = ["login", "signin", "credential"]
domain_peers  = [
    { term = "token",      weight = 0.8 },
    { term = "session",    weight = 0.8 },
    { term = "permission", weight = 0.7 },
    { term = "principal",  weight = 0.6 },
    { term = "identity",   weight = 0.6 },
]

[[cluster]]
seed = "token"
morphological = ["tokens"]
synonyms      = ["bearer", "credential", "secret"]
domain_peers  = [
    { term = "jwt",     weight = 0.9 },
    { term = "oauth",   weight = 0.8 },
    { term = "api_key", weight = 0.7 },
    { term = "expir",   weight = 0.6 }, # prefix — catches "expiry", "expired"
]
```

### Domain families

| Family | Terms |
|--------|-------|
| **auth** | login, token, session, bearer, jwt, oauth, permission, role, credential, expiry |
| **error** | exception, failure, panic, fatal, crash, abort, stacktrace, caused_by |
| **network** | timeout, connection, refused, socket, retry, unreachable, dns, tls, handshake |
| **database** | query, deadlock, constraint, migration, transaction, rollback, pool |
| **performance** | slow, latency, elapsed, duration, threshold, spike, queue, backpressure |
| **lifecycle** | startup, shutdown, init, ready, healthy, degraded, restart, reload |
| **resource** | oom, memory, disk, cpu, limit, exhausted, leak, gc, allocation |

## Negative prefix inference

When the query begins with a negative prefix (`un-`, `fail-`, `err-`, `invalid-`, `no-`), the traversal automatically biases toward the **error** and **failure** clusters:

```
"unauth"
  → prefix scan matches "unauthorized", "unauthenticated"
  → both are morphological children of seed "auth"
  → graph-walk proceeds from "auth"
  → negative prefix detected: weight-boost edges toward "error" / "failure" clusters
```

This means `unauth` at high greed naturally surfaces `forbidden`, `rejected`, `denied`, and `401` without any of those being explicitly linked to `unauth` in the ontology.

## Backwards resolution

Any term reachable from a seed at greed G must itself be able to reach that seed at some higher greed H ≤ 10.

**Example**: `auth` expands to `expiry` at greed 5 (forward edge weight 0.6). The reverse — `expiry → auth` — requires greed 9, because the reverse edge carries weight ≈ 0.3.

This asymmetry is encoded explicitly: every `domain_peer` edge defines both `weight` and `reverse_weight`. If `reverse_weight` is omitted it defaults to `weight * 0.4`.

```toml
[[cluster]]
seed = "auth"
domain_peers = [
    { term = "expiry", weight = 0.6, reverse_weight = 0.3 },
]
```

**Traversal mechanics**:
1. Start BFS from the input term.
2. Follow reverse edges whose weight ≥ `min_weight(greed)`.
3. Collect all seeds reachable within `max_depth(greed)` hops.
4. Include the forward expansions of those seeds as additional candidate terms.

**Testing invariant**: for every `(A, B)` pair where `A` expands to include `B` at greed G, `B` must expand to include `A` at some greed H where G < H ≤ 10. The `search_harness::prop_backwards_resolution` property test enforces this.

## Rust implementation

| Component | Crate | Notes |
|-----------|-------|-------|
| Prefix/trie lookup | [`fst`](https://crates.io/crates/fst) | Memory-mapped finite state transducer, fast prefix scans |
| Static ontology map | [`phf`](https://crates.io/crates/phf) | Compile-time perfect hash maps, zero runtime allocation |
| Graph traversal | std BFS | Gated by `depth <= max_depth && weight >= min_weight` |
