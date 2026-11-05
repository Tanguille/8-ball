# 8-ball conventions

- Follow [docs/rust-style.md](docs/rust-style.md) for descriptive identifiers, import placement and order, blank lines before exits, and domain-appropriate types.
- Use Rust 2024 and the stable toolchain.
- Enable the clippy pedantic, nursery, and style lints under `-D warnings`.
- Put module `foo` in `src/foo.rs` and its children in `src/foo/bar.rs`. Never use `mod.rs`.
- Use `thiserror` enums in library modules. Use `anyhow` only in `main.rs`.
- Use `format!("{name}")` with inlined arguments.
- Before you use a crate API, make sure that it matches the source in `~/.cargo/registry/src/*/<crate>-<version>/`. Never guess.
- Write the failing test first (TDD).
- Tests use wiremock as a scripted fake model, so no default test needs a real model.
- Write comments for non-obvious constraints only.
- Send logs to stderr through tracing. stdout carries JSON results and MCP JSON-RPC only.
- Ids are SQLite INTEGER rowids, not UUIDs. People type them by hand, as in `8ball label 42 yes`.
- Write commits in Commitizen style, with no AI attribution. Stage files by path.
- Before you say that work is done, run `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings`, and `cargo nextest run`.
- For the live feedback loop, run `dev/models.sh up`. Then run `cargo nextest run --test live --run-ignored only --no-capture`.
- The live stack uses the same llama.cpp image and pinned models as the cluster.
- Never loosen a live assertion without asking.
- Write docs in the Plain mode of [SimpleEnglish](https://github.com/AminBlg/SimpleEnglish). Keep docs short.
