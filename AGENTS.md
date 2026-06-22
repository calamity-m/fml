# fml

A tui tool built to assist overburdened developers triage issues by finding needles in log output haystacks

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:

- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them instead of picking silently.
- If a simpler approach exists, say so. Push back when warranted.
- Don't silently expand into wiring, integrations, or adjacent work that wasn't requested.
- If something is unclear, stop, name what's confusing, and ask.

## 2. Guidelines

- Test-only helpers or methods are rejected
- Do not refactor code to allow for testability without asking the user first
- Overengineering is a sin and premature optimization is the root of all evil but low quality code and committed shortcuts are just as bad
- Complex UI features should be self-tested for verification via any zellij or tmux use skills

## 3. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:

- "Add validation" -> "Write tests for invalid inputs, then make them pass"
- "Fix the bug" -> "Write a test that reproduces it, then make it pass"
- "Refactor X" -> "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:

```text
1. [Step] -> verify: [check]
2. [Step] -> verify: [check]
3. [Step] -> verify: [check]
```

Strong success criteria let you loop independently. Weak criteria require clarification.

## 4. In-Code Documentation

**Public API must be documented. Internal logic should explain the why.**

For public Rust modules, traits, structs, enums, functions, and constants:

- Use rustdoc comments: `//!` for modules and `///` for items.
- Describe what the item is for and any non-obvious parameter, return, or concurrency constraints.
- If the types make everything clear, a one-liner is enough.

For internal code, comment the why, not the what:

- Event ordering, async cancellation, terminal lifecycle, and store/search invariants earn a short comment.
- Keep comments short. Delete comments that merely restate the code.

## 5. Key Decisions

- The app is an async terminal UI built on tokio, crossterm, and ratatui.
- `App::event_loop` is the central reducer loop; keep event ordering changes deliberate and tested.
- `RingBufferStore` assigns monotonic sequence IDs while retaining only the configured capacity.
