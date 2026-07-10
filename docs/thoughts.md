# Direction note: what makes fml worth more than `stern > file && nvim`

> Status: **captured, not started.** This is a parked decision, not a plan.
> Recorded 2026-06-13. Revisit when there's appetite to build.

## The honest problem

As a _viewer_, fml is at rough parity with dumping stern to a file and opening
it in nvim. nvim handles ~1M lines fine; ripgrep/fzf cover most of the search
UX. Most of fml's code so far is viewer polish — that is **not** the moat. The
one existing real edge is the json/logfmt/pattern normalizers (structured
fields), which today only feed an info pane and are otherwise underused.

The tool's reason to exist is not "better viewer." It's:
**turn a live multi-pod mess into a shareable answer.**

Driving pain (user's words): _issues that resolve themselves once you find the
relevant logs and copy-paste them to a person._ The output of triage is a
curated, de-noised, timestamp-sorted excerpt handed to a human. That artifact is
the value moment, and neither stern (can't curate) nor nvim (can't merge a flat
file across sources by timestamp) produces it.

## Three candidate directions

1. **Cross-source timestamp correlation** — "show everything across all sources
   within ±Ns of this line, in real time order." Deepest moat. Today the store
   is arrival-ordered with no global sort.
2. **The triage deliverable (export)** — select window + source set + active
   filter → clean markdown/text artifact to clipboard/file. Smallest lift, maps
   directly to the stated pain.
3. **Live signal surfacing** — auto-flag error clusters, restarts/OOM, rare/new
   messages. Most speculative; no clear success criterion yet.

## Decisions made

- **Keep arrival order as the live/follow default. Do NOT make timestamp order
  global.** Sorting a moving stream by timestamp inserts late-arriving lines
  (backfill bursts, Docker/WSL2 batching) _above_ the cursor and shoves the
  viewport — it breaks "follow the tail." Timestamp order is also partly a lie
  (cross-node clock skew is routinely seconds) and undefined for raw files (no
  trustworthy per-line timestamp). The README's "no global sort" punt is correct
  for the live store.

- **Two modes, not one ordering:**
  - Live / follow → **arrival order** (stable, append-only). Unchanged.
  - Investigation → **timestamp order on a frozen slice** only. Because you've
    stopped following, the viewport-shove problem disappears — you sort a
    snapshot, not a moving stream.

- **#1 and #2 are one piece of work, not two.** The deliverable _is_ a
  timestamp-sorted cross-source merge of a frozen slice. The merge engine is the
  _useful core_ of #1 (frozen-slice version only — not the full live global
  sort). The export is the user-facing surface on top of that engine.

- **Sequencing:** #2 (with its merge engine) is the **experiment** that answers
  the open question "is fml actually more useful than nvim?" Build it first,
  scoped tight. The rest is the reward for passing the experiment.

- **SQLite / persistence for "way more lines" is deferred, not dead.** Likely
  sunk-cost right now: capacity isn't the triage bottleneck (finding + handing
  over the needle is), and nvim already proves big-file viewing is solvable.
  Build it only after #2 proves the tool earns a foundation. It then becomes the
  natural substrate for full live correlation + cross-session recall ("pull up
  Tuesday's incident").

- **Scope warning:** "export **and analysis**" — keep #1/#2 to export/curation
  (falsifiable: did the workflow change?). "Analysis" = #3 = speculative; do not
  let it balloon the experiment.

## Non-goals for the first build

- #1 as a _live_ feature (correlate while still following) — same engine, later
  surface, reopens the viewport-shove problem. Fast-follow if the experiment lands.
- #3 signal surfacing — out until the proven workflow asks for it.
- SQLite / persistence — deferred per above.

## Open questions to settle BEFORE planning the deliverable

The whole value lives here; unanswered = the plan is a blank:

- **Contents:** raw lines, or extracted fields? which fields?
- **Format:** markdown table? fenced text? something pasteable into Slack/Jira?
- **Selection:** how is the slice chosen — freeze + window + source set + active
  filter? what's the interaction?
- **Destination:** clipboard, file, or both?
- **Redaction:** any? (secrets/PII in prod logs)

Next step when picking this back up: grill the deliverable definition, then turn
the answers into a bigplan.
