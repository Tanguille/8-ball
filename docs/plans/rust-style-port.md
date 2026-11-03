# Mesh Organiser Rust style port

Status: Complete. Branch: `feat/v1`. Worktree: `/home/tanguille/Documents/PriveProjecten/8-ball`.

Port `mesh-organiser/docs/rust-style.md` into `docs/rust-style.md`, adapting Tauri/error/build examples to 8-ball and preserving its stronger module-layout and lint rules. Link from AGENTS.md and apply to all Rust source and tests. No dependency, schema, wire-format, public API or behavioral changes.

## Process instructions

Verify branch and worktree before work. Update status after each step. Normally pause for confirmation and suggest a continuation prompt between steps; the owner's instruction to port and apply authorizes completing all steps now. After the last step consolidate the lasting conventions into docs/rust-style.md and AGENTS.md; retain this file as verification evidence. For interruption, continue with: “Verify feat/v1 and the worktree, read docs/plans/rust-style-port.md, then finish the remaining style and validation steps.”

## Steps and ownership

- [x] Read source guide and project conventions; verify clean branch and pull.
- [x] Port Rust guide with 8-ball adaptations and link from AGENTS.md.
- [x] Parallel style pass A: src/main.rs, src/service.rs, src/store.rs, src/http.rs, src/mcp.rs.
- [x] Parallel style pass B: remaining src/*.rs and src/backend/*.rs; src/lib.rs already conforms.
- [x] Parallel style pass C: all 21 tests/*.rs files, preserving all payloads/assertions.
- [x] Review diff for behavior changes, run fmt, Clippy and full regression suite; all 22 live tests passed with unchanged assertions.
- [x] Commit by explicit paths; clean worktree check follows the local commit.

Each agent owns only its listed files. Parent owns documentation and commits. Rules: full-word locals except conventional indices/errors; imports at file top grouped std/external/current crate/super; merge common prefixes, no renamed imports except Trait as _; qualify singleton type/module uses, retain required method-resolution traits; preserve public field names, comments and data literals; blank lines before returns/final expressions where useful. Expand compressed code that rustfmt cannot expand. No new abstractions, mass type migration, lint suppressions or tests that merely assert formatting.

## Verification

Baseline `RUSTC_WRAPPER= cargo nextest run`: 103/103 passed, 22 live tests skipped (run 93acde25-1140-492e-abfa-2847d6107370, 0.877s). After the style pass, `cargo fmt --all` and `RUSTC_WRAPPER= cargo clippy --all-targets -- -D warnings` passed; the full regression suite again passed 103/103 (0.762s). Existing regression tests cover this style-only refactor; no behavior is added.

Independent core diff review found only style issues (SQL chain/doctor closure indentation, singleton imports and final-expression spacing), all addressed. Local literal comparison against HEAD shows only inline format-capture renames to match local variables; data literals, SQL, error text, assertions and fixture files remain unchanged. Production Clippy passes with warnings denied.

Guide adaptations: replace Tauri/ApplicationError examples with existing 8-ball error/API conventions; retain signed SQLite ids and current public types; permit required method-resolution trait imports; let rustfmt place cfg attributes; retain 8-ball's stronger no-mod.rs rule. Frontend and mesh-specific Git/agent workflow policies are outside this Rust style port.

Final command results:

```text
cargo fmt --all
exit 0
RUSTC_WRAPPER= cargo clippy --all-targets -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.36s
RUSTC_WRAPPER= cargo nextest run
Nextest run ID dad5d625-aaf8-4fde-b31b-37985db6f211
Summary [0.762s] 103 tests run: 103 passed, 22 skipped
RUSTC_WRAPPER= cargo nextest run --test live --run-ignored only --no-capture
Nextest run ID 4e987967-1b48-4ec6-822f-4f67c5f23944
Summary [16.368s] 22 tests run: 22 passed, 0 skipped
greeting holdout: precision=1, coverage=0.85, decided=17/20, correct=17
storage-risk holdout: precision=1, coverage=0.95, decided=19/20, correct=19
git diff --check
exit 0
```

Live tests used the same temporary rootless Podman user/SELinux override documented in the SQLx migration report. Temporary containers/network were removed afterward; temporary audit scripts, override and logs are removed before committing. No dependencies, fixture data, schema or deployment configuration changed. No new container build or external scan was needed for naming/import/formatting changes; no push performed.
