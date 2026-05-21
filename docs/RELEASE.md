# Release Guide

Use this checklist when cutting a new release.

## Versioning

- This is a Rust workspace.
- The canonical version is in `Cargo.toml` under `[workspace.package]`.
- `fml/Cargo.toml` uses `version.workspace = true`; do not add a separate package version there.
- `Cargo.lock` records the `fml` package version and should be updated as part of the release.

For a normal release:

1. Confirm the bump type with the requester: patch, minor, or major.
2. Update `[workspace.package].version` in `Cargo.toml`.
3. Run `cargo check --workspace` to refresh `Cargo.lock`.
4. Check the diff before testing.

## Snapshot expectations

The TUI renders the application version in the footer. A version bump changes snapshots that include the footer.

When bumping the version, update snapshot files under `fml/tests/snapshots/` that still contain the old `vX.Y.Z` footer. During the `v0.2.0` to `v0.3.0` release, multiple snapshots needed only this version-text update.

Useful check:

```sh
rg 'vOLD.VERSION' fml/tests/snapshots
```

Do not accept unrelated snapshot changes. Release snapshot changes should only replace the rendered version string.

## Verification

Run the normal workspace tests before staging:

```sh
cargo test --workspace
```

Also run the integration feature set before pushing, because the pre-push hook runs it and can catch additional snapshot drift:

```sh
cargo test --workspace --features integration
```

The local hooks also run build, fmt, clippy, and tests during commit/push. If a hook finds release-related snapshot drift, update only the affected release snapshots and amend the release commit.

## Commit, push, and tag

Stage only release-related files:

- `Cargo.toml`
- `Cargo.lock`
- release-related snapshot updates, if the rendered version changed

Commit format:

```sh
git commit -m "chore(release): vX.Y.Z"
```

Push `main` after tests pass:

```sh
git push origin main
```

This repository uses version tags (`v0.0.1`, `v0.0.2`, etc.). Create and push an annotated tag for the release:

```sh
git tag -a vX.Y.Z -m "release vX.Y.Z"
git push origin vX.Y.Z
```

## Things to avoid

- Do not stage unrelated working tree changes.
- Do not create a separate version in `fml/Cargo.toml`.
- Do not broadly accept snapshot changes; inspect that they are version-only changes.
- Do not push a tag before the release commit has pushed successfully.
