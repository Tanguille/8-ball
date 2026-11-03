# 8-ball conventions

- Follow [docs/rust-style.md](docs/rust-style.md): descriptive identifiers, import placement/order, blank lines before exits, and domain-appropriate types.
- Read docs/plans/8-ball-v1.md "READ THIS FIRST" before any work.
- Rust 2024, stable toolchain. Lints: clippy pedantic + nursery + style under -D warnings.
- Module foo lives in src/foo.rs, children in src/foo/bar.rs. Never mod.rs.
- thiserror enums in library modules, anyhow only in main.rs.
- format!("{name}") with inlined args.
- Verify crate APIs in ~/.cargo/registry/src/*/<crate>-<version>/ before using them. Never guess.
- TDD: failing test first. Tests use wiremock as a scripted fake model; no test needs a real model.
- Comments explain non-obvious constraints only.
- Logs go to stderr via tracing. stdout carries JSON results and MCP JSON-RPC only.
- Ids are SQLite INTEGER rowids (typed by hand in `8ball label 42 yes`), not UUIDs.
- Commits: Commitizen style, no AI attribution, stage files by path.
- Checks before saying done: cargo fmt --all; cargo clippy --all-targets -- -D warnings; cargo nextest run.
- Live feedback loop: dev/models.sh up, then cargo nextest run --test live --run-ignored only --no-capture.
  Same llama.cpp image and pinned models as the cluster. Never loosen a live assertion without asking.

