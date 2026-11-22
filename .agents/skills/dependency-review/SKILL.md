---
name: dependency-review
description: Where each dependency of 8-ball is consumed, which versions move together, and what to check per kind. Use on Renovate PRs.
---

# Dependency review

## Cargo crates

Every version lives in `[workspace.dependencies]` in the root `Cargo.toml`. Members inherit it with `workspace = true`, so search `*/src` for the symbols a note changes.

- `parquet` is used only in `bench/src/main.rs`.
- `sqlx` is used through `sqlx::migrate` in `store/src/lib.rs`, with `sqlite-bundled`.
- `rmcp` and `schemars` move together. The manifest says `schemars` must stay the version `rmcp` re-exports, so flag a bump of one that leaves the other behind.

CI runs `cargo clippy --locked -- -D warnings` and `cargo nextest run`. Any Cargo bump releases a new image, because `image.yaml` runs on `Cargo.lock`.

## llama.cpp image

The digest sits once in the `x-llama` anchor in `dev/compose.yaml`, and its comment says it matches the deployed model servers. Check the notes for the server flags in the `command:` lines, such as `--reranking`, `--pooling`, `--cache-ram`, `--reasoning-budget` and `-fa`. CI runs the live tests and the benchmark gate against `bench/baseline.json` on the PR. Renovate automerges this file once checks pass, so report a flag change as a finding.

## Helm

`aqua:helm/helm` in `.mise.toml` and the `version` of `azure/setup-helm` in `.github/workflows/image.yaml` must be the same version. The `common` dependency in `charts/eightball/Chart.yaml` must match `Chart.lock`. The chart uses the bjw-s values API, so read the notes for renamed values in `charts/eightball/values.yaml`.

## Tools and actions

`.mise.toml` tools run only in `lint.yaml`. Actions are pinned by SHA with a version comment, and the same SHA appears in several workflows. Check that the SHA matches the tagged release and that every workflow moved. `zizmor` audits the workflows.

## Not managed here

`Dockerfile` uses `rust:1-alpine` and `rust-toolchain.toml` uses `stable`, both unpinned.
