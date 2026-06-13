# BIGPLAN: Mode-dependent fuzzy search stability

## Plan Overview

Fuzzy search currently re-ranks forever: the worker re-emits a fresh top-N every
tick, even after the search is confirmed, so the results view shifts under the
user and `n`/`N` jumps land on a moving target. This effort ties search
stability to the pane's existing follow flag: a following (TAIL) pane gets a
live search that keeps re-ranking pinned to the newest matches, while a
non-following pane gets a stable search — a bounded scan over everything
retained at dispatch time that completes and then stops. Scrolling up through
live results breaks follow and freezes the search seamlessly; `F`/`:tail` goes
live again and a new `:refresh` re-snapshots a frozen search in place. Done
means: retained results never shift unless the pane is following, and the
live/frozen boundary is the same follow boundary the rest of the TUI already
uses.

## Risks

- **Frozen entry shifts** — the core guarantee fails if a non-following pane
  applies incomplete bounded emissions or lets the final bounded result move
  the selected row. Mitigation: non-following bounded searches apply progress
  during the scan but suppress entry-list replacement until `complete = true`,
  and the complete replacement preserves the selected seq when it is still
  retained. Live searches with too few rows to move do not freeze, so they keep
  updating until there is something real to navigate. Watch-for: any row or
  cursor jump in a frozen results view, excluding retained-only eviction below.
- **Bounded scan churn on large stores** — freezing and non-following typing
  both create bounded scans over retained history; doing a full abort + rescan
  for every keypress makes large stores feel laggy. Mitigation: include a
  bounded-search coalescing/reuse path in the protocol deliverable before
  enabling per-keystroke bounded dispatches, and verify full-capacity typing
  and freeze latency manually.
- **Completion signal dependency** — held entry updates release only on the
  bounded request's accepted `complete = true` emission. Mitigation: tie holds
  to the active query/request identity, clear holds on every superseding
  dispatch, and test that stale or mismatched completions cannot release or
  replace the held view.
- **Query identity ripple** — `Query::Fuzzy(String)` becomes a struct variant
  carrying the bound, and `Pane::apply_result` drops results whose query
  doesn't equal `active_query`. Every construction/match site (engine dispatch,
  tick-rate selection, worker, pane, render, tests) must agree on the new
  shape, and equality-including-bound is load-bearing for a specific window:
  the engine's request-id check only discards old-worker emissions *after* the
  freeze's `Search` event is processed, but the pane's `active_query` changes
  synchronously at input time — an old emission already queued ahead of the
  `Search` event passes the request-id check and is dropped only by query
  inequality. A missed construction site compiles into silently dropped
  results. Mitigation: make the change in one deliverable, lean on exhaustive
  `match`, and cover same-term/different-bound races with pane tests.
- **Retained-only stability** — the stability guarantee covers entries still
  retained by the ring buffer; heavy ingest can evict held rows before the
  bounded rescan completes. Mitigation: state the retained-only guarantee in
  target semantics and acceptance checks, and treat eviction churn as outside
  the no-shift promise rather than silently preserving evicted entries.
- **Bound freshness** — every bounded fuzzy dispatch depends on capturing the
  store high seq before the search event is sent. Mitigation: centralize bound
  capture at pane/TUI dispatch sites and cover freeze, frozen typing, refresh,
  and clone re-dispatch with tests that newer entries do not leak into the
  bounded result.
- **Cursor displacement on freeze** — the bounded rescan's top-N display cap
  can differ slightly from the live worker's (scores tie-break differently at
  the cap edge), so the entry under the cursor can drop out of the displayed
  subset. Mitigation: preserve the selected seq on the complete replacement
  whenever it is still present; otherwise fall back to the nearest retained
  match and cover the invariant with a pane test.

## Plan Details

### Current behavior (verified in code)

- `/` calls `Pane::begin_search`, which forces `follow = false`
  (`fml/src/tui/pane.rs:475`). Every search is detached from tail today.
- The fuzzy worker (`fml/src/search/fuzzy.rs`) holds a `ScanState` across
  ticks, scans incrementally, emits ranked top-N every `tick_rate`, and after
  completing a scan keeps rescanning new entries forever. There is no upper
  seq bound.
- `SearchEvent::Result` already carries `complete: bool`, and the fuzzy worker
  emits `complete = true` on the final emission of each snapshot scan —
  including unbounded ones (a new snapshot starts when bounds drift). However,
  the reducer (`fml/src/search.rs` `handle_search_event`) only logs `complete`;
  it is **not** passed into `Pane::apply_result`. The hold-until-complete
  handoff needs it plumbed through.
- After `Enter` confirms, `active_query` stays `Fuzzy`, so the worker keeps
  running and the results view keeps re-ranking — the jolt being fixed.
- The engine (`fml/src/search.rs`) is latest-wins per pane: a new
  `SearchEvent::Search` aborts the old worker and bumps `request_id`; stale
  request ids are dropped in the reducer; `Pane::apply_result` additionally
  drops results whose query != `active_query` (this covers emissions queued
  ahead of the superseding `Search` event — see the query-identity risk).
- `search_return_seq` is the existing abandon mechanism: `begin_search`
  records the cursor and `abandon_search` (Esc) restores it.
- All freeze trigger sites (`move_cursor`, `jump_to`, `jump_hit`,
  `enter_follow`, command handlers) already receive a `SearchCtx` carrying
  retained `(low, high)` store bounds — no plumbing needed for the freeze
  bound.
- The `TAIL` badge and statusline already key off `pane.follow`
  (`fml/src/tui/render.rs:396,409`), so live search shows TAIL for free once
  `begin_search` stops forcing follow off.

### Target semantics

| | Following (TAIL) | Not following (frozen) |
|---|---|---|
| `/` + typing | live: unbounded query, re-ranks every tick, cursor pinned to newest match | bounded at dispatch-time high seq; scan completes, then worker stops |
| scroll up in results | freezes (only if the cursor actually moves) | normal cursor movement, no re-dispatch |
| `Enter` (confirm) | stays in live results view; hit list keeps extending as new matches arrive | hit list frozen at the snapshot |
| `n` / `N` | first jump is cursor motion → freezes at jump time; the extended hit list (including post-confirm matches) is what it walks | walks the frozen hit list |
| `Esc` (abandon/leave) | restores the tailing stream (follow preserved, `Query::Tail`) | restores the anchored stream at `search_return_seq` (today's behavior) |
| `F` / `:tail` | — | goes live on the active search (results view), or tails the stream if no search |
| `:refresh` | no-op (already live) | active fuzzy query: re-dispatches it bounded at the *current* high seq, stays frozen; otherwise a notice |

- **Freeze trigger**: any motion in a results view that actually moves the
  cursor off its row — `j`/`k`, paging, `n`/`N`, `Enter`-on-hit. If the
  cursor can't move (0–1 matches, already at the boundary, or not enough
  results to scroll the pane), the keypress is a no-op and the search stays
  live so the underfilled view can keep updating.
- **Stable means bounded-and-complete**: entries newer than the freeze bound
  are never scored, but the scan over older *still-retained* entries runs to
  completion, so a freeze taken mid-scan still ends up covering all retained
  history. Non-following bounded searches update progress while scanning but
  do not replace the displayed entry list until the accepted complete
  emission; the complete replacement preserves the selected seq if it is
  still retained. Ring eviction can shrink that retained set — see Gotchas.
- **Live hit extension exists for the freeze**: while confirmed-and-live, the
  hit set keeps growing and the cursor rides the newest match; the payoff is
  that when motion eventually freezes the search, `n`/`N` walk a hit list that
  includes everything that arrived after confirm.
- **Frozen indicator**: no new marker. Absence of the TAIL badge *is* the
  frozen signal, consistent with stream views; the results header keeps
  showing term + progress.

### Critical Files

- `fml/src/event.rs` — `Query::Fuzzy(String)` grows an upper bound field.
- `fml/src/search/fuzzy.rs` — worker honors the bound: skip entries above it,
  emit `complete = true` once the scan reaches it, then return instead of
  entering the rescan loop.
- `fml/src/search.rs` — engine dispatch/tick-rate match arms for the new
  `Fuzzy` shape; plumb `complete` into `Pane::apply_result`.
- `fml/src/tui/pane.rs` — `begin_search` / `update_search` /
  `apply_result` / `move_cursor` / `enter_follow` / `confirm_search` /
  `abandon_search` / `clone_into`: the whole mode-dependent lifecycle lives
  here.
- `fml/src/tui.rs` — `/` entry, search-mode keys, `:refresh` command, help
  text.
- `fml/src/tui/render.rs` — statusline already follow-driven; no frozen
  marker is added (no TAIL badge = frozen).
- `docs/MODAL_REDESIGN.md` — the "Search is per-pane grep + jump" section and
  keymap/command table describe the old semantics; update alongside.

### Gotchas

- `Pane::move_cursor` currently sets `follow = false` unconditionally before
  computing the new index. The freeze-only-on-real-movement rule means follow
  must only break when `new_idx != idx` — and this changes stream-view
  behavior too (a one-entry tailing stream no longer breaks follow on `k`).
  That is consistent and desirable; the existing
  `move_cursor_breaks_follow_and_anchors_history` test still passes because
  real movement still breaks follow.
- **`holding_for_complete` must clear on every new dispatch** — typing a
  character, `F`/`:tail`, `:refresh`, `Esc`, filter changes: anything that
  dispatches supersedes the held handoff. A hold that only clears on the happy
  path (`complete` arrives) silently swallows all partial emissions of the
  next query — a frozen-forever pane that looks exactly like the bug being
  fixed. Clearing inside `Pane::dispatch` covers every path at once.
- During a non-following bounded scan, progress-only updates still apply
  (header shows the scan advancing); only the *entries* are held until the
  accepted `complete = true` emission. This includes per-keystroke bounded
  dispatches from a frozen pane, so the implementation must coalesce/reuse
  rapid bounded searches rather than relying on partial entry updates for
  responsiveness.
- **Live worker teardown is dispatch-driven, by design**: every exit from a
  live search dispatches a replacement query (`Esc` → `Tail`,
  `Enter`-on-hit / `n`/`N` into stream → `History`, `:clear` →
  `results_to_stream`), and the engine's latest-wins abort kills the fuzzy
  worker. Pane close is covered by the existing closed-pane routing path,
  which cancels the engine slot. No new teardown mechanism is needed — but any
  future exit path that *doesn't* dispatch would leak a perpetual worker.
- **Ring eviction during a freeze**: under heavy ingest, old entries in the
  held view can be evicted before the bounded rescan reaches them, so the
  `complete` emission can contain fewer old entries than the held view showed.
  The stability guarantee is retained-only: when the ring evicts a held entry,
  the final frozen result drops it instead of preserving a copy.
- The freeze bound lives **inside the query** (`Query::Fuzzy.until_seq`);
  there is no separate pane field. `clone_into` therefore carries the bound by
  copying `active_query` (today it resets it to `None`). The clone keeps the
  copied view and hit state, resets `holding_for_complete` (results are routed
  by pane id, so a clone mid-hold would never receive the release emission),
  and — having no running worker — stays a static frozen view until the user
  interacts, at which point `:refresh`/`F` re-dispatch from the copied term.
- Same-term fuzzy queries with different bounds are distinct identities.
  `Fuzzy { term: "err", until_seq: None }`, `Fuzzy { term: "err", until_seq:
  Some(100) }`, and `Fuzzy { term: "err", until_seq: Some(150) }` must not
  accept each other's emissions, even when request-id ordering alone would let
  one through.
- Bound capture belongs at the pane/TUI dispatch edge, before the
  `SearchEvent::Search` is emitted. The worker should treat `until_seq` as an
  immutable input, not re-read the store high bound later.
- A bounded worker that finishes leaves a completed `JoinHandle` in
  `SearchClientState.running_handle`; aborting a finished handle is a no-op,
  so the existing abort-on-new-dispatch path needs no change.
- `jump_hit` and `jump_to` already set `follow = false`; jumping from a live
  results view into stream context therefore lands frozen, which matches the
  target semantics — but the search itself must freeze too (bound captured at
  jump time) or the still-running live worker will keep mutating
  `live_hit_seqs`.

### Pseudo-code / Sketches

```text
Query::Fuzzy { term: String, until_seq: Option<u64> }   // None = live

fuzzy worker:
  bound = until_seq.unwrap_or(u64::MAX)
  scan only entries with seq <= bound
  emit top-N each tick as today (complete=true on the final snapshot emission,
  as the protocol already does)
  on scan complete:
    if until_seq.is_some(): emit complete=true, return (worker exits)
    else: keep incremental rescan loop (current behavior)

Pane::begin_search:        // no longer forces follow = false
  search_return_seq = cursor_seq      // existing abandon-restore anchor

Pane::dispatch(query, ctx):
  holding_for_complete = false        // any new dispatch releases a held handoff
  ... existing dispatch ...

Pane::update_search(term):
  until = follow ? None : Some(capture_high_bound(ctx))
  dispatch_fuzzy_with_bounded_coalescing(Fuzzy { term, until })

Pane::freeze_search(ctx):  // called when motion breaks follow in Results
  follow = false
  bound = capture_high_bound(ctx)
  view.entries.retain(seq <= bound)        // instant visual stability
  dispatch(Fuzzy { term, until: Some(bound) })
  holding_for_complete = true              // set after dispatch clears it

Pane::apply_result(Fuzzy, complete):       // complete newly plumbed from reducer
  if !follow && fuzzy.until_seq.is_some():
      apply progress to view header        // scan visibly advances
      if !complete: return                 // frozen entries stay held
  if holding_for_complete && complete:
      holding_for_complete = false
  selected_seq = cursor_seq
  ... existing seq-sort / cursor / view logic ...
  if !follow && selected_seq is still displayed/retained: cursor = selected_seq
  if follow: cursor = newest match (pin to bottom)
  if follow && confirmed(term): hits = live_hit_seqs   // live hit extension

Pane::abandon_search(ctx):                 // Esc from search input
  cursor_seq = search_return_seq.take()
  if follow: dispatch(Query::Tail)         // was live: back to tailing stream
  else:      dispatch(History { middle: cursor_seq })   // today's behavior

Pane::enter_follow(ctx):   // F / :tail
  follow = true
  if view is Results with active fuzzy term:
      dispatch(Fuzzy { term, until: None })
  else:
      dispatch(Query::Tail)

:refresh
  if active fuzzy && !follow:
      dispatch(Fuzzy { term, until: Some(capture_high_bound(ctx)) })
      // one complete re-rank, then stable again
  else: notice ("nothing to refresh" / already live)
```

## Deliverables

### Deliverable 1. Bounded fuzzy queries in the worker and protocol

Change `Query::Fuzzy(String)` to a struct variant carrying `until_seq:
Option<u64>` and teach the fuzzy worker to honor it: entries above the bound
are never scored, the scan emits `complete = true` when it reaches the bound,
and a bounded worker then returns instead of entering the perpetual rescan
loop. `until_seq: None` preserves today's live behavior exactly. Also plumb
the existing `complete` flag from the reducer into `Pane::apply_result` — it
is currently dropped there and the freeze handoff (Deliverable 3) keys off it.
This is the protocol foundation everything else stands on; it touches
`event.rs`, `search.rs` dispatch/tick-rate arms, `fuzzy.rs`, and every
existing construction site (pane, tui, tests) in one pass so the compiler
flushes out stragglers. It also adds the minimal bounded-search
coalescing/reuse needed to avoid a full abort + rescan for every frozen
keypress. Acceptance: existing fuzzy worker tests pass unchanged with
`until_seq: None`; new worker tests prove bounded scans exclude newer entries
and terminate after the complete emission, and engine/pane coverage proves
rapid bounded updates converge on the latest query without running every
intermediate scan.

- [x] Reshape `Query::Fuzzy` to `{ term, until_seq }` and fix all construction/match sites
- [x] Add a single bound-capture helper/path for fuzzy dispatch sites so `until_seq` is fixed before `SearchEvent::Search` is emitted
- [x] Plumb `complete` from `handle_search_event` into `Pane::apply_result`
- [x] Fuzzy worker skips entries with `seq > until_seq` during scan and rescan setup
- [x] Bounded worker emits final `complete = true` and exits (no rescan loop)
- [x] Add minimal bounded-search coalescing/reuse so rapid frozen typing does not spawn a full scan for every intermediate term
- [x] Worker test: entries appended after dispatch never appear in bounded results
- [x] Worker test: bounded worker task finishes after the complete emission
- [x] Worker/engine test: rapid bounded updates converge on the latest query without applying intermediate entry lists
- [x] Worker test: unbounded query still picks up new entries on subsequent emits (existing `new_entries_appear_on_subsequent_emit` still green)

### Deliverable 2. Live tail search

Make `/` inherit the pane's follow state instead of forcing it off. In a
following pane the search stays live: unbounded query, results re-rank each
tick, cursor pinned to the newest match (mirroring tail-stream cursor
pinning), TAIL badge showing via the existing follow-driven statusline.
`Enter` confirms in place — the pane stays in the live results view and the
confirmed hit list keeps extending from `live_hit_seqs` on each emission, so
the hit list a later freeze inherits includes post-confirm matches. `Esc`
becomes follow-aware: abandoning a live search returns to the tailing stream
(follow preserved); abandoning from a non-following pane restores the anchored
position as today — both paths dispatch a stream query, which also tears down
the fuzzy worker via latest-wins. In a non-following pane, `/` dispatches
bounded queries per keystroke (fresh bound each dispatch) and confirm freezes
the hit list as today. Acceptance: searching from TAIL shows new matches
arriving live with the cursor riding the newest one; Esc lands back on the
live tail; searching from normal mode produces a result set that stops
changing once the scan completes.

- [x] `begin_search` no longer clears `follow`; `update_search` picks `until_seq` from follow state
- [x] Live fuzzy `apply_result` pins the cursor to the newest match while following
- [x] Confirmed live search keeps `hits` synced to `live_hit_seqs` on each new emission
- [x] `abandon_search` dispatches `Tail` when following, anchored `History` otherwise
- [x] Pane test: `/` from a following pane dispatches unbounded fuzzy and keeps `follow = true`
- [x] Pane test: `/` from a non-following pane dispatches bounded fuzzy
- [x] Pane test: confirmed live search extends `hits` when a later emission adds matches
- [x] Pane test: confirm from a non-following bounded search leaves `follow = false`, keeps the bounded query, and does not switch to live reranking
- [x] Pane test: Esc from a live search restores the tailing stream with `follow = true`
- [x] Statusline check: live results show the TAIL badge + progress; frozen results show no badge (no new marker) — badge is purely `pane.follow`-driven (`render.rs:396`), verified by the follow-state pane tests

### Deliverable 3. Freeze on scroll with seamless handoff

Scrolling up through live results breaks follow and freezes the search:
capture the store high bound, trim the displayed list to `seq <= bound`
immediately, dispatch the bounded query, and hold the trimmed view until the
bounded request's `complete` emission replaces it — no visible rebuild
(eviction churn excluded). During the hold, progress updates still render and
the held entries stay navigable. The freeze only fires on real cursor
movement: if the cursor can't move (short list, already at the boundary, or
not enough results to scroll the pane), the keypress is a no-op and the search
stays live. The first `n`/`N` and `Enter`-on-hit from a live results view are
cursor motion and freeze the same way, capturing the bound at jump time. Any
new dispatch — typing, `F`, `:refresh`, `Esc` — clears the old hold so a
superseding query can never be silently swallowed; if that superseding query
is still bounded/non-following, its entries are held until its own complete
emission. Once frozen, cursor motion inside results never re-dispatches.
Acceptance: pressing `k` in a busy live search lands on a stable list with no
flicker; pressing `k` with one match does nothing and matches keep streaming
in; typing during a handoff renders the new term's progress and final results
normally.

- [x] `move_cursor` breaks follow only when the cursor index actually changes
- [x] Freeze path: trim view to bound, dispatch bounded query, hold entries until `complete`, render progress meanwhile
- [x] `Pane::dispatch` clears `holding_for_complete` on every dispatch
- [x] Complete bounded replacement preserves the selected seq when it is still retained; otherwise falls back to nearest retained match
- [x] Stale live-worker emissions after freeze are dropped: pane test for an emission queued with the current request id but the pre-freeze (unbounded) query
- [x] Pane tests: same-term fuzzy emissions with different bounds are rejected unless the bound matches `active_query` (unbounded→bounded freeze and older-bounded→newer-bounded refresh)
- [x] First `n`/`N` and `Enter`-on-hit from a live results view freeze the search at jump time (`n`/`N` via `freeze_search`; `Enter`-on-hit via `results_to_stream`, whose `History` dispatch tears down the live worker)
- [x] Pane test: scroll-up with an unmovable/underfilled cursor keeps `follow = true` and dispatches nothing
- [x] Pane test: partial emissions during freeze handoff don't replace the held view; the complete one does
- [x] Pane test: typing during a handoff clears the old hold; the new bounded term applies progress immediately and entries only on complete
- [x] Pane test: freeze captures the high bound before dispatch and excludes entries appended afterward
- [x] `clone_into` copies `active_query` (the bound rides in `until_seq`) and resets `holding_for_complete`, so a split of a frozen search stays frozen

### Deliverable 4. Going live again: `F`/`:tail` on a search and `:refresh`

`enter_follow` becomes view-aware: with an active fuzzy search it re-dispatches
the term unbounded and pins to the newest matches (staying in the results
view); on a plain stream it tails the stream as today. Add `:refresh`
(re-snapshot in place): with a frozen fuzzy search it re-dispatches the same
term bounded at the current high seq — one re-rank, then stable again — and
stays in normal mode; while following, or with no active fuzzy query, it shows
a notice instead. Update the help overlay, `docs/MODAL_REDESIGN.md` search
section, and the command table to describe the live/frozen model. Acceptance:
F in a frozen results view starts showing new matches again; `:refresh` pulls
in matches that arrived since the freeze without entering tail.

- [x] `enter_follow` re-dispatches the active fuzzy term unbounded when in a results view
- [x] `:refresh` command parser/routing added for normal-mode command handling
- [x] `:refresh` behavior: frozen pane with active fuzzy query re-dispatches bounded at current high seq; following/no-fuzzy contexts show a notice
- [x] Pane/tui test: F in frozen results goes live; `:refresh` re-snapshots without setting follow
- [x] Pane/tui test: `:refresh` captures a fresh high bound and excludes entries appended after that dispatch
- [x] Help overlay documents live/frozen search, TAIL/no-TAIL meaning, and `:refresh`
- [x] `docs/MODAL_REDESIGN.md` search section, keymap, and command table updated for live/frozen search semantics and `:refresh`

## Issues

- **2026-06-13 — agent:claude (implementation)** — All four deliverables
  implemented; 306 `cargo test -p fml` pass, no new clippy warnings, `cargo
  fmt` clean. Notes on decisions made while building: (1) bounded-search
  coalescing lives in the `app.rs` event loop, which drains the queued
  search-event batch and calls a pure, unit-tested `coalesce_search_events`
  that drops any same-target `Search` superseded by a later `Search`/`Cancel`
  before a worker is ever spawned. (2) Per the freeze-handoff gotcha,
  `update_search` from a non-following pane sets `holding_for_complete` too, so
  per-keystroke bounded dispatches hold the displayed entries until their own
  `complete` (coalescing keeps this from running every intermediate term). (3)
  `break_follow` (column motion) was gated to `View::Stream` so `h`/`l` in a
  live results view no longer leaves the view — only row motion / `n`/`N`
  freeze. (4) `Enter`-on-hit relies on `results_to_stream`'s `History` dispatch
  to tear down the live worker (latest-wins) rather than an explicit freeze.
- **2026-06-13 — agent:claude (adversarial review)** — Plan reviewed by 2
  adversarial sub-agents (Risks & Assumptions, Completeness & Scope). 12
  distinct findings; 12 merged into plan after user decisions. Most
  significant: frozen bounded searches now hold entry updates until complete,
  the stability guarantee is explicitly retained-only, cursor preservation is
  required on complete replacements, and bounded-search coalescing/reuse is in
  scope for large-store typing.
- **2026-06-12 — agent:claude (adversarial review)** — Plan reviewed by 2
  adversarial sub-agents (Risks & Assumptions, Completeness & Scope). 12
  distinct findings; 12 merged into plan. Most significant: the
  `holding_for_complete` handoff needed an exit on every new dispatch (not
  just the happy path), the missing Esc/cancel semantics are now defined
  (follow-preserving abandon), and the n/N-in-live-results contradiction was
  resolved by the user: the first `n`/`N` freezes at jump time.
- **2026-06-12 — agent:claude** — Plan created from a grilled brief; all seven
  clarifying decisions (search inherits follow, bounded-scan freeze, mode-split
  confirm, freeze-only-on-real-movement, `:refresh` action, view-aware F,
  stay-in-live-results on confirm) were confirmed by the user as recommended.
