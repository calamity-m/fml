---
name: git-commit
description: Commit staged changes.
---

# /git-commit — Commit staged changes

Run CI, review staged/unstaged changes, and create a conventional commit.

## Steps

### 1. Run CI

```bash
make ci
```

Capture the test/lint summary (pass/fail counts, warnings). If the output looks
worse than a clean run (new failures, more warnings than before), **stop and ask
the user to inspect before continuing**. Do not commit over a broken CI.

### 2. Review changes

Run these in parallel:

```bash
git diff --stat          # unstaged changes
git diff --cached --stat # staged changes
git status --short       # overall picture
```

If there are **unstaged changes that look related to the staged changes**, point
them out and ask the user whether to add them before committing. Do not add
files silently — always ask.

### 3. Understand the diff

```bash
git diff --cached
```

Read the full staged diff to understand what changed and why.

### 4. Generate a commit message

Format: `<type>(<scope>): <short imperative summary>`

Types: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `perf`, `ci`

Scope: the layer or subsystem touched (e.g. `store`, `search`, `kubernetes`,
`docker`, `normalizer`, `export`, `ui`, `ci`, `deps`). Omit parens if the
change is truly cross-cutting.

Rules:
- Subject line ≤ 72 characters, lowercase after the colon, no trailing period
- Body (optional): wrap at 72 chars, explain *why* not *what*
- Footer: always end with `[vibed]` on its own line, nothing else in the footer

Example:

```
feat(search): add negative prefix bias toward error cluster

When a query starts with un-/fail-/err-, automatically weight-boost
edges toward the error and failure clusters so that unauth surfaces
forbidden, rejected, and 401 without explicit ontology links.

[vibed]
```

### 5. Confirm and commit

Show the commit message to the user and ask for approval or edits before
running `git commit`. Once approved:

```bash
git commit -m "$(cat <<'EOF'
<message here>
EOF
)"
```

Do not amend existing commits unless the user explicitly asks.
