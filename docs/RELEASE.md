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

## Verification

Run the normal workspace tests before staging:

```sh
cargo test --workspace
```

Also run the integration feature set before pushing, because the pre-push hook runs it and can catch additional snapshot drift:

```sh
cargo test --workspace --features integration
```

The local hooks also run build, fmt, clippy, and tests during commit/push.

## Commit, push, and tag

Stage only release-related files:

- `Cargo.toml`
- `Cargo.lock`

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
- Do not push a tag before the release commit has pushed successfully.
