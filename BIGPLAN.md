# BIGPLAN: Profiles & Source Blocking (#5 + #6)

## Plan Overview

Land named producer **profiles** in config (#5) and layer **source blocking** on top (#6) as a single coordinated effort. Profiles give users a TOML-defined startup bundle of producers (`profiles.<name>.producers`), selectable via a new `--profile` CLI flag, while existing `--producer` flags continue to work and override matching entries inside the active profile. Source blocking is exposed per-producer via a `blocked = "<regex>"` key plus a `skip_istio = true` shortcut, with a global `--skip-istio` CLI escape hatch. The implementation must use **`SourceBlock(ed/ing)`** terminology end-to-end so a future line-level redaction feature can extend the same matcher abstraction without churn. "Done" means: a user can write a profile, launch with `--profile foo`, override one of its producers from CLI, and trust that blocked containers/pods never appear in the source selector or the log store.

## Risks

- **Config schema additions land in a published TOML format** — existing user `config.toml` files have no `profile` / `[profiles.*]` keys; new fields must be `#[serde(default)]` and the absence of a profile must be a no-op, otherwise we silently break every user. Mitigation: default `Option<String>` profile field, default empty `BTreeMap<String, ProfileConfig>`, regression test loading an old-shape config.
- **CLI override ambiguity** — `--producer kubernetes` is ambiguous when a profile contains two kubernetes producers (different namespaces). Mitigation: define a clear rule (CLI override matches by `(kind, disambiguator)`; if multiple match without a disambiguator, error out with the candidate list rather than guessing). Document and test the rule.
- **Source-id stability vs. block matching** — block regex matches *id* OR *name*, but kube/docker source ids are derived from container/pod identity. If id formatting changes, regex written against ids breaks silently. Mitigation: matcher takes the resolved `Source` and tests both fields explicitly; document which strings are tested and add tests that pin the id format.
- **Producer-internal block timing** — k8s and docker discover sources asynchronously; the block check must run before any `SourceFound` is sent and before any entry is forwarded for that source. A naive "filter at the event bus" approach would still leak `SourceFound`. Mitigation: matcher lives inside each producer's discovery path, not at the bus. Tests assert no `SourceFound` is emitted for blocked entries.
- **Future line-level blocking** — naming and trait shape have to accommodate per-entry filtering later without renaming. Mitigation: name the abstraction `SourceBlock` (the *thing* being blocked is the source today) and structure the matcher so a sibling `LineBlock` can be added without touching `SourceBlock` callers. Avoid leaking the `SourceBlock` name into per-line code paths now.
- **`--skip-istio` precedence** — needs to compose with profile config (additive, never subtractive). Mitigation: `--skip-istio` is a *strengthening* flag — it always adds `istio-proxy` to the block list of every kube/docker producer in effect, regardless of whether `blocked` already covers it.
- **Per-producer failure isolation** — invalid regex in one profile producer must not prevent unrelated producers from starting (preserves #5 done-criterion). Mitigation: compile regex at producer construction; on failure, log + skip that producer and continue.
- **Multiple `SourceFound` emission sites** — kube and docker discovery may not be a single linear stream (initial list + watch reconnects + per-namespace scopes). If the block check is added at one site but missed at another, the matcher silently leaks. Mitigation: Deliverable 4/5 explicitly audit all `SourceFound` emission sites and either funnel them through one helper or add the check at every site.
- **Per-entry leaks (ordering invariant)** — a leaked `SourceFound` is a one-tick UI blip; a leaked `StoreEvent` lives in `RingBufferStore` until evicted. Mitigation: log-tail tasks must be spawned *only after* the block check passes; the `is_source_blocked` call must precede every `tx.send(SourceFound)` and every `tx.send(StoreEvent)` for that source.
- **`regex` crate dependency footprint** — `blocked` accepts user-supplied regex from config. The `regex` crate is already DoS-safe (no catastrophic backtracking), but if it isn't already a workspace dep, this adds one. Default 10 MB compiled-size limit is fine for the expected use case. Mitigation: confirm dep at start of Deliverable 3; do not customize the size limit unless a real case shows up.
- **Failure-mode consistency** — config-shape errors (unknown profile name, malformed TOML) are fatal at startup; per-producer compile errors (bad regex) are isolated (log + skip); CLI ambiguity (`--producer kubernetes` matching two profile entries) is fatal. State this principle once in the gotchas so reviewers and future contributors don't re-litigate it per-deliverable.

## Plan Details

### Critical Files

- `fml/src/config.rs` — top-level `Config` struct; add `profile: Option<String>` and `profiles: BTreeMap<String, ProfileConfig>`.
- `fml/src/config/` — new `producer.rs` submodule for `ProfileConfig` / `ProducerConfig` enum and the `SourceBlockConfig` shape.
- `fml/src/producer.rs` — `ProducerSpec` enum + parser. Producers carry the matcher via the **paired tuple form** `(ProducerSpec, SourceBlock)` (or a small `ResolvedProducer` newtype) — `ProducerSpec` itself stays as-is so the existing CLI parser is unchanged. Also home of the `LogProducer` trait.
- `fml/src/log.rs` — defines `Source { producer, id, display_name, group }`. The matcher's contract is pinned to these field names; if they change, `SourceBlock` callers must follow.
- `fml/src/producer/kubernetes.rs` — primary integration site for blocking: discovery loop must consult the matcher before emitting `SourceFound` and before forwarding entries.
- `fml/src/producer/docker.rs` — same as kube: container discovery + log stream gating.
- `fml/src/producer/file.rs` / `fml/src/producer/fake.rs` — accept a (no-op or trivially-matched) `SourceBlock` to keep the producer interface uniform; `file` ignores `blocked` per spec.
- `fml/src/main.rs` — `Cli` struct gets `--profile`, `--skip-istio`; resolution from (config, profile, --producer overrides, --skip-istio) into the final `Vec<(ProducerSpec, SourceBlock)>` lives here (or in a new `fml/src/cli.rs` if main grows).
- `fml/src/app.rs` — `App::new` signature: today takes `Vec<ProducerSpec>`; will take `Vec<(ProducerSpec, SourceBlock)>` (or a new `ResolvedProducer` struct).
- `fml/tests/` — integration tests for profile loading, CLI override resolution, and producer-level block behavior.

### Gotchas

- `config.rs` already double-loads when `config_dir` is overridden — profile resolution must run *after* the second load, not the bootstrap one.
- `handle_producer_event` in `producer.rs` auto-enables every newly arrived `SourceFound` in the source selector. Blocking must short-circuit *before* `SourceFound` reaches the bus, not at the reducer, otherwise we'd leak the source into `state.tui.source_selector` for one tick.
- `ProducerSpec::parse` is used by both CLI and (future) config loading; if config also calls `parse`, the CLI's `splitn(2, ':')` behavior is reused for free, but config's typed TOML form (`type = "kubernetes"`, `namespace = "..."`) is more ergonomic. Recommendation: keep `ProducerSpec::parse` for CLI strings only; have a separate `ProducerConfig -> ProducerSpec` conversion for TOML.
- The `blocked` regex is matched against **`source.id` OR `source.display_name`** — make this explicit in docs and tests; users will assume one or the other otherwise.
- `skip_istio = true` is a producer-level shortcut; `--skip-istio` is a global shortcut. Both compose by *adding* `istio-proxy` to the producer's matcher; neither removes anything.
- Blocks are static for the process lifetime — no runtime mutation API, no UI affordance to unblock. Document this in the `SourceBlock` rustdoc.
- Future: per-line blocking will live as a sibling concept (`LineBlock`?). Today's `SourceBlock` should not assume it owns the future namespace; keep its method names focused (`is_source_blocked(&Source) -> bool`).
- **Blocked sources emit no events at all** — no `SourceFound`, no `SourceLost`, no `StoreEvent`. Downstream code (source selector, store stats, search) must already cope with sources it has never heard of being absent; the contract is "the producer behaves as if the source never existed."
- **Multiple `SourceFound` call sites** — before adding the block check, grep `kubernetes.rs` and `docker.rs` for every `SourceFound` and `StoreEvent` send site. The check must guard each, or all sends must be funnelled through one local helper.
- **Single design for spec+matcher** — chosen: paired tuple `(ProducerSpec, SourceBlock)` (or `struct ResolvedProducer { spec, block }`). `ProducerSpec` is **not** extended to carry the matcher because (a) `ProducerSpec` is reused by the CLI parser which has no block info, (b) the matcher is a *compiled* artifact and the spec is a value type.
- **Disambiguator per producer kind** for CLI overrides:
  - `demo` — none, and `demo` is **repeatable**: a profile may contain multiple `demo` entries, and `--producer demo` *appends* a new demo producer rather than overriding any existing one.
  - `file` — path. `--producer file:/abs/path` matches by exact string equality with the profile's `file = "..."`. No path canonicalisation magic.
  - `docker` — none, and **at most one** docker producer per profile (a second is a config error). `--producer docker` overrides the single docker entry if present, otherwise appends.
  - `kubernetes` — namespace string (or "no namespace" matches the profile's no-namespace entry). `--producer kubernetes` with two namespaced kube entries in the profile is ambiguous → error with the candidate list.
- **`--producer` override drops the profile entry's `blocked` / `skip_istio`** — by design. A CLI `--producer` is treated as a brand-new producer config block, not a partial patch. This pushes users toward `--profile` + config for durable settings and keeps `--producer` as a quick ad-hoc tool. `--skip-istio` is the one CLI knob that does compose, because it is a global escape hatch rather than a per-producer override.
- **`skip_istio` / `--skip-istio` use substring match** — the matcher tests whether `source.id` or `source.display_name` *contains* the literal `"istio-proxy"` (case-sensitive). This handles real container/pod ids like `istio-proxy-abc123` and pod-prefixed forms like `productpage/istio-proxy`. The `blocked = "..."` regex is independent and full-regex.
- **Missing profile is a hard error** — both `--profile foo` and `config.profile = "foo"` with no matching `[profiles.foo]` table abort startup with a clear message listing the profiles that *do* exist. No fallback, no warn-and-continue. Renaming a profile is a deliberate user action and silent fallback would mask it.
- **`FML__*` env vars and the new fields** — `FML__PROFILE=foo` works for free via the existing `Environment` source. `FML__PROFILES__*` for nested profile maps is ugly but technically possible; we *do not* officially support env override of `[profiles.*]` content for v1. Document this.
- **Keep resolution and compilation separate** — profile/CLI resolution produces `Vec<(ProducerSpec, SourceBlockConfig)>`; `SourceBlock` compilation happens only after the matcher exists. This keeps Deliverable 2 buildable without depending on Deliverable 3 internals.

### Pseudo-code / Sketches

#### Config structs

```text
// fml/src/config/producer.rs
struct ProfileConfig {
    producers: Vec<ProducerConfig>,
    // future: top-level profile keys (env, default-theme, etc.) go here
}

enum ProducerConfig {
    Demo,
    File   { file: PathBuf },
    Docker { #[serde(default)] block: SourceBlockConfig },
    Kubernetes {
        #[serde(default)] namespace: Option<String>,
        #[serde(default)] block: SourceBlockConfig,
    },
}
// serde tag = "type", rename_all = "lowercase"

#[derive(Default)]
struct SourceBlockConfig {
    blocked: Option<String>,   // regex, matched against id OR display_name
    skip_istio: bool,          // adds istio-proxy to the matcher
}

// fml/src/config.rs (additions, both #[serde(default)])
struct Config {
    ...
    profile: Option<String>,
    profiles: BTreeMap<String, ProfileConfig>,
}
```

#### SourceBlock matcher

```text
// fml/src/producer/source_block.rs
pub struct SourceBlock {
    regex: Option<Regex>,         // compiled from `blocked`
    substrings: Vec<String>,      // includes "istio-proxy" when skip_istio is on
}

impl SourceBlock {
    pub fn from_config(cfg: &SourceBlockConfig, force_skip_istio: bool)
        -> Result<Self, regex::Error> { ... }

    pub fn none() -> Self { Self { regex: None, substrings: vec![] } }

    pub fn is_source_blocked(&self, source: &Source) -> bool {
        // Substring match — handles real ids like "istio-proxy-abc123" and
        // pod-prefixed forms like "productpage/istio-proxy".
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
```

#### CLI override resolution

```text
// pseudo, lives in main.rs (or fml/src/cli.rs)
fn resolve_producer_configs(config: &Config, cli: &Cli) -> Result<Vec<(ProducerSpec, SourceBlockConfig)>> {
    // 1. Start from the active profile's producers (or empty if no --profile / config.profile).
    // 2. For each --producer CLI string:
    //      parse into a CliSpec { kind, disambiguator: Option<String> }
    //      find the producer in the working set whose (kind, disambiguator) matches:
    //        - kubernetes:bob   -> kubernetes producer with namespace == "bob"
    //        - kubernetes       -> the only kubernetes producer; if >1, error
    //        - docker / file:.. -> match on kind, disambiguator (path) for file
    //      if found: replace with CLI version (CLI loses any `blocked` from profile entry — explicit override)
    //      if not found: append as a new producer
    // 3. If --skip-istio: walk the working set; for every kube/docker entry,
    //      OR `skip_istio = true` into its SourceBlockConfig before compiling.
    // 4. Return raw block configs; SourceBlock compilation happens after
    //      SourceBlock exists and can isolate per-producer regex failures.
}
```

#### Producer integration (kubernetes example)

```text
// inside KubernetesProducer's discovery task
for pod_event in stream {
    let source = build_source(&pod_event);
    if self.source_block.is_source_blocked(&source) {
        continue; // never emit SourceFound, never tail logs
    }
    tx.send(ProducerEvent::SourceFound(source)).await?;
    spawn_log_tail(source, tx.clone(), self.cancel.child_token());
}
```

Docker mirrors this in its container discovery loop. `file` and `fake` accept a `SourceBlock` for interface uniformity but `file` documents that `blocked` is currently a no-op (per spec: "does not support blocked yet"); `fake` ignores it too.

## Deliverables

### Deliverable 1. Profile config schema + loader

Extend `Config` with `profile: Option<String>` and `profiles: BTreeMap<String, ProfileConfig>`, plus a typed `ProducerConfig` enum (`type = "demo" | "file" | "docker" | "kubernetes"`) under `[[profiles.<name>.producers]]`. Both new top-level fields default to absent so existing user configs keep working. Add a `Config::resolve_profile(&self, name: Option<&str>) -> Result<Option<&ProfileConfig>>` helper that errors when a requested profile name is missing from the map.

- [x] Add `fml/src/config/producer.rs` with `ProfileConfig`, `ProducerConfig`, `SourceBlockConfig`.
- [x] Add `profile` and `profiles` fields to `Config` with `#[serde(default)]`. Use `BTreeMap` for deterministic test iteration.
- [x] Implement `Config::resolve_profile` returning `Result<Option<&ProfileConfig>, FmlError>`.
- [x] Implement `impl TryFrom<&ProducerConfig> for ProducerSpec` (typed TOML → spec) plus the matching `SourceBlockConfig` extraction. The two outputs (`spec`, `block_config`) are paired explicitly; do NOT extend `ProducerSpec` itself.
- [x] Confirm the `regex` crate is a workspace dep (or add it); pin a version that aligns with whatever else uses it. No custom size limit.
- [x] Add unit tests: load TOML with a profile, load TOML without any profile, load TOML with multiple profiles, load TOML referencing an unknown profile name (hard error with a "did you mean…" / available-profiles message), `ProducerConfig::try_into` round-trip per kind.
- [x] Validation: a profile with multiple `docker` entries is a config error; a profile with multiple `demo` entries is allowed (demo is repeatable). Test both.
- [x] Regression test: an existing minimal `config.toml` (no profile keys) still deserializes to `Config::default()`-equivalent.

### Deliverable 2. CLI `--profile` + `--producer` override semantics

Wire `--profile <name>` and adjust `--producer KIND[:ARG]` to act as an override against the active profile. Rules: CLI producers replace profile producers that match by `(kind, disambiguator)`; CLI producers with no profile match are appended; ambiguous overrides (e.g. `--producer kubernetes` with two kube entries in the profile) error out with the candidate list. Resolution lives in main (or a new `fml/src/cli.rs` if main grows past comfort). The result is a `Vec<(ProducerSpec, SourceBlockConfig)>` — `SourceBlock` is *compiled* in Deliverable 3, not here.

- [x] Add `--profile` to `Cli`. If `--profile` is unset, fall back to `config.profile`.
- [x] Implement `resolve_producers(...)` per the pseudo-code (no `SourceBlock` compilation yet — return raw `SourceBlockConfig` alongside each spec).
- [x] Tests: profile-only resolution; CLI-only resolution (no profile); profile + CLI override match (kube namespace); profile + CLI override append (no matching profile entry); ambiguous override error (two kube namespaces, bare `--producer kubernetes`); override drops profile entry's `blocked`/`skip_istio` (CLI is treated as a brand-new producer config block); `--profile foo` with no matching profile aborts with available-profiles message; `--producer demo` appends rather than overrides; multi-`docker` in a single profile fails config validation.
- [x] Document precedence rules in `--profile` and `--producer` help text. Pick docs target (README.md is the existing pattern in this repo) and add a short profile/blocking section there.

### Deliverable 3. `SourceBlock` matcher abstraction

Introduce `fml/src/producer/source_block.rs` containing `SourceBlock` (compiled matcher) and conversion `SourceBlock::from_config(&SourceBlockConfig, force_skip_istio: bool) -> Result<Self, regex::Error>`. `is_source_blocked(&Source)` tests literal names then regex against `source.id` and `source.display_name`. Naming is deliberate: this module reserves `SourceBlock` for source-level filtering; future line-level filtering ("redact lines matching X") will be a sibling type, not a generalization of this one.

- [x] Add the module, struct, and `from_config` / `none` / `is_source_blocked` methods.
- [x] Compile `blocked` regex once; surface `regex::Error` so the caller can per-producer-isolate failures.
- [x] Add a `ResolvedProducer` newtype or equivalent paired form that carries `(ProducerSpec, SourceBlock)` after compilation.
- [x] Update `App::new` to take compiled resolved producers. Update every call site — `main.rs`, integration tests, `app.rs` self-tests — to pass the new shape.
- [x] Implement per-producer compile-error isolation: when `SourceBlock::from_config` fails for one entry, log a warning identifying the profile + producer and drop that producer from the resolved list; do not abort the others.
- [x] `skip_istio = true` (or `force_skip_istio: true` from `--skip-istio`) adds `"istio-proxy"` as a **substring** to test against both `source.id` and `source.display_name`.
- [x] Unit tests: empty matcher matches nothing; regex matches id; regex matches display_name; substring match for `istio-proxy-abc123`; substring match for `productpage/istio-proxy`; combined regex + skip_istio (both still apply); invalid regex returns error.
- [x] Rustdoc on `SourceBlock` documents (a) static-for-process-lifetime semantics, (b) match-against-id-OR-name, (c) producer is responsible for never emitting events for a blocked source, (d) sibling future of `LineBlock`.

### Deliverable 4. Wire `SourceBlock` into the kubernetes producer

`KubernetesProducer` accepts a `SourceBlock` at construction (via `KubernetesProducer::new(namespace, source_block)` or a builder). Discovery loop calls `is_source_blocked(&source)` before emitting `SourceFound`; blocked sources are silently skipped. Pod log tasks are never spawned for blocked pods. New pods discovered after startup follow the same path (so the matcher is consulted on every discovery cycle, not just initial enumeration).

- [x] Extend constructor to accept `SourceBlock`.
- [x] **Audit**: grep `kubernetes.rs` for every `SourceFound` and `StoreEvent` send site, document them in a comment, and ensure every site is guarded by `is_source_blocked`. Prefer funnelling through one local helper `try_announce_source(&self, source) -> Option<Source>` to make the invariant unmissable.
- [x] Add the block check immediately before `SourceFound` is emitted; do not spawn the per-pod log-tail task for blocked pods.
- [x] Test: producer with `blocked = "^istio"` does not emit `SourceFound` for a pod whose `display_name` starts with `istio`.
- [x] Test: producer with `skip_istio = true` blocks the istio sidecar (see Decisions for the exact match semantics chosen).
- [x] Test: a pod that appears post-startup (simulated second discovery tick) is also blocked.
- [x] Test: blocked sources produce zero entries in the `LogStore`.

### Deliverable 5. Wire `SourceBlock` into the docker producer

Same shape as Deliverable 4 but applied to docker container discovery. `--skip-istio` and `skip_istio = true` should match the docker container named `istio-proxy` (the typical istio sidecar container name); regex `blocked` matches container id or display name.

- [x] Extend `DockerProducer` constructor to accept `SourceBlock`.
- [x] **Audit**: same as kube — enumerate every `SourceFound` / `StoreEvent` send site in `docker.rs` and funnel through one `try_announce_source` helper.
- [x] Add the block check before `SourceFound` emission in container discovery; do not start log-streaming for blocked containers.
- [x] Test: `blocked = "_postgres_"` blocks containers whose name contains `postgres`.
- [x] Test: `skip_istio = true` blocks `istio-proxy`.
- [x] Test: post-startup container appearance is also blocked.
- [x] Test: blocked containers produce zero entries in the `LogStore`.

### Deliverable 6. `--skip-istio` global flag + `skip_istio` shortcut composition

Final wiring: `--skip-istio` on CLI ORs `skip_istio = true` into every kube and docker entry's `SourceBlockConfig` *before* `SourceBlock::from_config` is called. `file` and `fake` producers ignore `--skip-istio` (it is meaningless for them; documented). Compose, never override: a profile entry already setting `blocked = "^istio"` plus `--skip-istio` results in both the regex and the literal `istio-proxy` being active.

- [x] Add `--skip-istio` boolean to `Cli`.
- [x] In `resolve_producers`, after profile/CLI merge, walk all kube + docker entries and OR `skip_istio = true` into their `SourceBlockConfig`.
- [x] Test: `--skip-istio` with a profile that has no block config blocks `istio-proxy` for all kube/docker producers.
- [x] Test: `--skip-istio` composes with a producer-level `blocked = "_postgres_"` (both still apply).
- [x] Test: `--skip-istio` is a no-op for `file` and `fake` producers (no error, no behavioral change).

### Deliverable 7. Tests, docs, and integration polish

Round out the cross-cutting story: end-to-end integration test that loads a TOML profile, applies a CLI override and `--skip-istio`, constructs the app, and asserts the resolved producer list. Update README / user docs with: profile config example (mirror the user's example block), `--profile` / `--producer` precedence table, `--skip-istio` semantics, and a note that blocking is static for the process lifetime.

- [x] Integration test in `fml/tests/` covering profile load → CLI override → `--skip-istio` → producer construction.
- [x] README / user docs: profile example block, precedence rules, blocking semantics, restart-to-unblock note.
- [x] Update `--producer` help text to mention profile interaction.
- [x] Verify per-producer failure isolation: regression test where one profile producer has an invalid regex and other producers still start.
- [x] Run `cargo test --workspace` and `cargo clippy --workspace --all-targets` clean for new code (existing drift in `producer/file.rs` and `producer/normalizer/logfmt.rs` is out of scope per CLAUDE.md §6).

## Issues

- **2026-05-07 — agent:codex (adversarial review)** — Plan reviewed locally through Risks & Assumptions plus Completeness & Scope lenses. 3 findings; 1 merged into plan. Main merged fix separates profile/CLI resolution from `SourceBlock` compilation so Deliverable 2 does not depend on Deliverable 3 internals.
<!-- newest first; entries are dated and signed -->

- **2026-05-07 — agent:claude (adversarial review)** — Plan reviewed by 2 adversarial sub-agents (Risks & Assumptions, Completeness & Scope). 20 findings; ~14 merged into plan as new risks/gotchas/tasks (App::new wiring, ProducerConfig→Spec conversion, emission-site audit, ordering invariant, env-var policy, disambiguator table, single design choice for spec+matcher). 6 findings deferred to user as decision items — see "Decisions pending" below.

- **2026-05-07 — agent:claude (decisions resolved)** — All six pending decisions answered by Mark:
  1. `skip_istio` / `--skip-istio` use **substring** match against `source.id` and `source.display_name` (matches `istio-proxy-abc123`, `productpage/istio-proxy`, etc.).
  2. CLI `--producer` **drops** the profile entry's `blocked` / `skip_istio` — override is a brand-new producer config block. `--skip-istio` is the one composing flag.
  3. **Hard error** when a referenced profile is missing (both `--profile` and `config.profile`); message lists available profile names.
  4. `demo` is **repeatable** in a profile; `docker` is single-only (multi is a config error). `kubernetes` continues to allow multiple namespaced entries.
  5. `file` and `fake` carry a (no-op / regex-only) `SourceBlock` for **interface uniformity**.
  6. Docs target: **README.md**.
