# ROADMAP

This document tracks larger product and engineering efforts for `fml`. It is intentionally higher-level than `BIGPLAN.md`: use this to see what is planned, why it matters, and what each effort needs before it can be called done.

## Status Key

- **Planned** - Not started beyond notes or design sketches.
- **Designed** - Has a concrete plan or bigplan ready for implementation.
- **In Progress** - Code is being actively changed.
- **Done** - Implemented, documented where needed, and verified.

## Roadmap Items

### 1. Preview Pane Modes

**Status:** Designed
**Plan:** `BIGPLAN.md`

Extend the preview pane beyond the current surrounding-context mode. The planned modes are surrounding, field-matched, and expanded. Field-matched follows selected field values such as request ids or trace parents across retained logs, while expanded keeps surrounding context but wraps long log lines.

Done means:

- Users can switch preview modes with the default `ctrl+p` binding.
- Field-matched mode has a usable picker for fields on the selected log entry.
- Field-matched preview can show matching logs across sources.
- Expanded mode wraps long preview lines without breaking anchor visibility.
- Search, state, picker, and rendering behavior have focused tests.

### 2. Historical Sourcing

**Status:** Planned

Support ingesting historical logs before switching into live-follow behavior. This should cover Kubernetes, file, and Docker producers so users can open `fml` after an incident starts and still pull relevant earlier context into the retained store.

Done means:

- Kubernetes producer can request useful prior pod/container logs before following live logs.
- File producer can load historical content from the target before tailing new writes.
- Docker producer can request prior container logs before following live stdout/stderr.
- Historical ingestion preserves source identity and ordering well enough for tail, history, fuzzy search, and preview flows.
- Producer configuration can bound historical reads so startup does not accidentally ingest unbounded data.

### 3. Compressed File Support

**Status:** Planned

Extend the file producer to read compressed log files. This is primarily for historical and rotated logs where the active file may only contain the newest entries.

Done means:

- File producer can read common compressed log formats selected for first support, such as gzip.
- Compressed file reads reuse the existing normalizer path.
- Unsupported compression formats fail clearly instead of being treated as plain text.
- Tests cover compressed input, decode errors, and interaction with historical file ingestion.

### 4. Directory Support for File Producer

**Status:** Planned

Allow file producer configuration to point at directories, not only individual files. Directory support should make it practical to ingest log sets such as `/var/log/my-app/` or rotated log directories without manually listing every path.

Done means:

- File producer can discover files under a configured directory.
- Directory discovery has explicit include/exclude rules or documented defaults.
- New files appearing in a watched directory can be picked up when live-follow mode is active.
- Source identities remain stable across discovered files.
- Tests cover initial directory discovery, new file discovery, and ignored files.

### 5. Configurable Keybindings

**Status:** Planned

Complete user-configurable keybindings through config. Some config scaffolding already exists under `[tui.keybindings]`, but runtime handling still needs to use the resolved config consistently across global actions, popup-local actions, focused widgets, help, and status labels.

Leads: keybinds.rs

Done means:

- Config can remap all intended non-reserved key actions.
- Reserved fallbacks are explicit and documented.
- Help and status surfaces reflect configured primary bindings.
- Invalid key specs fail with a clear config error.
- Existing hardcoded bindings such as source selector and preview mode switch are moved behind the config model where appropriate.

### 6. Profiles Config for Producers

**Status:** Planned

Add named profiles that bundle producer configuration. Profiles should let users define reusable startup contexts, such as a local file profile, a Docker profile, or a Kubernetes namespace profile, without spelling out every producer flag each run.

Done means:

- Config supports named profiles with producer definitions.
- CLI can select a profile explicitly.
- Existing `--producer` flags keep working.
- The merge/precedence rules between profiles and CLI producer flags are documented and tested.
- Producer construction still reports per-producer failures without preventing unrelated producers from starting.

### 7. Source Blocking via Producer Config

**Status:** Planned

Allow producer config to block noisy or irrelevant sources before they enter the UI and store. A primary example is blocking `istio-proxy` containers from Kubernetes or Docker-derived sources.

Done means:

- Producer config supports source block rules using stable source identity and/or producer-specific labels.
- Blocked sources do not appear in the source selector.
- Blocked source entries are not inserted into the log store.
- The implementation distinguishes persistent config-level blocking from temporary UI source filtering.
- Kubernetes and Docker producers have tests for common block cases such as `istio-proxy`.

### 8. Export Functionality

**Status:** Planned

Add a first export path for retained logs or selected log ranges. This should focus on a simple, reliable export target before integrating with external tools.

Done means:

- Users can export a defined set of logs, such as current visible results, selected context, or retained range.
- Export format is explicit, likely starting with JSON lines and/or plain text.
- Export preserves enough metadata to be useful: sequence, timestamp, level, source, message, and fields.
- Export errors are visible and do not destabilize the TUI.
- Tests cover format shape and range/result selection.

### 9. Export Integrations

**Status:** Planned

Build on core export functionality with workflow integrations for terminals and editors. Candidate targets include tmux, zellij, VS Code, and a user-selected editor command.

Done means:

- Core export can hand off to an external command or integration layer.
- tmux and zellij workflows are supported without assuming either is present.
- Editor launch behavior is configurable and handles paths with spaces.
- VS Code integration is treated as one optional target, not a hard dependency.
- Failures produce actionable messages instead of silent no-ops.

### 10. SQLite-Backed Log Store

**Status:** Planned

Add SQLite as an optional log store implementation alongside the current ring-buffer store. The goal is to support larger retained windows, persistence across short sessions if desired, and more efficient query shapes for future features without forcing SQLite onto simple runs.

Done means:

- Store config can choose between in-memory ring buffer and SQLite-backed storage.
- SQLite implementation satisfies the existing `LogStore` behavior or a deliberately revised store trait.
- Sequence ids remain monotonic and compatible with tail, history, fuzzy search, and preview search.
- Retention/capacity behavior is explicit for both store choices.
- Tests compare core store behavior across ring-buffer and SQLite implementations.

### 11. Query Box Commands

**Status:** Planned

Support command execution through the query box, similar to Vim commands or the VS Code command palette. The query box should remain the fast fuzzy-search surface, but gain a command mode for actions that are easier to invoke by name than by remembering a keybinding.

Done means:

- Query box can distinguish search input from command input using an explicit syntax or mode.
- Commands can trigger existing actions such as opening help, changing preview modes, exporting logs, or selecting profiles.
- Command discovery is usable, with command names and descriptions available in the UI.
- Invalid or incomplete commands provide clear feedback without disrupting the active search.
- Command handling is tested separately from fuzzy-search dispatch so search behavior does not regress.

## Dependencies and Sequencing

- Preview pane modes can proceed independently and already have a bigplan.
- Historical sourcing should land before compressed file and directory support are considered complete, because those file-producer features need to compose with startup history reads.
- Directory support should define source identity rules before profiles and source blocking rely on directory-discovered file sources.
- Configurable keybindings should land before treating `ctrl+p` and other new actions as long-term defaults.
- Profiles config should come before source blocking so block rules have a natural home.
- Core export should land before tmux, zellij, VS Code, or editor integrations.
- Query box commands should reuse the same action layer as configurable keybindings where possible, so commands and shortcuts do not diverge.
- SQLite store work should wait until current store/search contracts are stable enough to avoid reworking the trait twice.

## Open Questions

- Should profiles replace producer CLI flags in common workflows, or only supplement them?
- How much historical data should each producer load by default: no history unless configured, a line count, a byte count, or a time window?
- Which compressed file formats should the first file-producer pass support beyond gzip, if any?
- Should directory support recurse by default, or require explicit recursive configuration?
- Should source block rules match display names, stable source ids, producer metadata, or all of the above?
- Which export set should be first: visible log pane rows, selected preview context, fuzzy results, or an explicit retained sequence range?
- Should SQLite persistence be session-local by default, or should it require an explicit database path?
- Should commands use Vim-style `:` prefixes, a VS Code-style palette mode, or support both?
