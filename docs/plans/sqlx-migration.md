# SQLx storage migration

Created: 2026-11-03. Status: Complete.
Branch: `feat/v1`. Worktree: `/home/tanguille/Documents/PriveProjecten/8-ball`.

## Outcome and scope

Replace rusqlite with the latest stable SQLx (0.9.0, confirmed with Cargo and official documentation). Preserve SQLite data, ids, labels, calibration, cache scoping, transactions, permissions and CLI behavior. Make Store::open asynchronous and update every caller. PostgreSQL, pgvector, new configuration and generic repository frameworks are deferred; this migration supplies their foundation only.

## Process instructions

Verify branch/worktree before work. Update this plan after each step. Ordinarily pause for user confirmation between steps and suggest the next continuation prompt; the owner's explicit request to plan and then delegate implementation authorizes executing this entire migration without those pauses. After completion consolidate operational guidance into README and retain this plan as verification evidence. Use strict red/green/refactor/live/commit, read actual crate sources under ~/.cargo/registry before using APIs, and stage explicit paths without attribution. No push, deployment, global config edits or tool installation.

## Design

- Use SQLx SQLite with default features disabled and only Tokio runtime, SQLite and required migration features. No SQLx CLI installation. Check the resolved source for exact feature names.
- Use a single-connection SqlitePool initially to preserve serialized operations and avoid new SQLite write contention. Async initialization and SQLx workers replace the hand-written mutex/spawn_blocking layer. Bind all values; use typed query_as/query_scalar and integration tests, avoiding build-time database requirements.
- Keep file creation mode 0600 before connection creation so WAL/SHM remain private. Preserve WAL, NORMAL synchronous mode and five-second busy timeout explicitly.
- Preserve current schema and legacy AUTOINCREMENT upgrade transactionally. Never rebuild an already-AUTOINCREMENT table: its sqlite_sequence high-water mark can exceed surviving ids. Preserve existing calibration rows and indexes. Prefer a versioned embedded SQLx migration for baseline tables, with legacy compatibility conversion inside the same initialization transaction; inspect SQLx migration transaction APIs before choosing implementation details.
- Preserve nullable `spec IS ?`, newest-row cache behavior, atomic batch insert, label behavior, calibration upsert and retention boundaries. Malformed stored JSON remains an error, not a panic.
- Store methods remain unchanged except async open. Keep StoreError style; no connection details in user-facing logs beyond existing behavior.

## Execution and ownership

| Chunk | Write set | Dependencies | Wave |
|---|---|---|---|
| 1: regression contracts | tests/store.rs, Cargo.toml/Cargo.lock if needed for fixtures | reviewed plan | 1 |
| 2: SQLx storage | src/store.rs, migrations/, Cargo.toml, Cargo.lock, tests/store.rs | chunk 1 red evidence | 2 |
| 3: caller conversion | src/main.rs, tests/*.rs except store.rs | chunk 1 red evidence, agreed async open signature | 2 |
| 4: validation and docs | README.md, this plan, any reviewed integration fixes | chunks 2 and 3 | 3 |

Parent alone updates plan and commits. Agents have disjoint write sets in wave 2; they must not revert another agent's changes. Smaller available models implement bounded tasks; parent handles architecture and final review.

### Chunk 1 — regression contracts
- [x] Confirm baseline storage tests and latest release: 8/8 passed; cargo search confirms SQLx 0.9.0.
- [x] Convert storage tests to await Store::open and observe the intended compile failure before production changes: exit 101, E0277 `Result<Store, StoreError> is not a future`, 11 call sites.
- [x] Add independent pre-SQLx fixture coverage: legacy rowid database upgrade, existing AUTOINCREMENT database with deleted high ids, labels/calibration persistence and repeated open. Cover atomic batch rollback with an aborting trigger, concurrent cloned-store writes, missing labels, malformed JSON and file privacy where coverage is missing. Existing behavior tests may already pass; record that honestly.

### Chunk 2 — storage implementation
- [x] Inspect cached SQLx 0.9.0 sources; use sqlite-bundled, runtime-tokio and migrate only; remove rusqlite entirely.
- [x] Implement async connection setup, compatibility migration and parameterized storage operations; convert fixtures to SQLx with independent static legacy SQL.
- [x] Run focused storage suite: 14/14 passed; focused Clippy passed.

### Chunk 3 — integration callers
- [x] Await open in main and every test/helper, propagating async only where required. Preserve all assertions and model requests.
- [x] Compile all tests: `RUSTC_WRAPPER= cargo check --all-targets` passed; CLI/REST/MCP runtime verification is part of the full suite below.

### Chunk 4 — finish
- [x] Independent gpt-6.1-sol review: no actionable findings; verified nested savepoint and cancellation/rollback handling against SQLx source.
- [x] `cargo fmt --all`; `RUSTC_WRAPPER= cargo clippy --all-targets -- -D warnings` passed; `RUSTC_WRAPPER= cargo nextest run` passed 103/103 (22 live tests skipped by default). Nextest run a28ea28d-918d-49ee-b386-e9b4db4693f8, 0.756s test execution.
- [x] Run local model stack and full ignored live suite; `RUSTC_WRAPPER= cargo nextest run --test live --run-ignored only --no-capture` passed 22/22, zero skipped, 25.423s. Assertions unchanged. See temporary Podman override below.
- [x] Existing Dockerfile builds successfully; release compilation completed in 2m35s. Local image `8ball:sqlx-review` runs `--version` successfully as configured user 65532.
- [x] Document storage behavior and upgrade compatibility in README; record command outcomes here. All migration/caller changes are committed together to preserve a compiling revision; final clean status is verified after the commit.

## Rollback

Use a consistent SQLite backup before deployment (including WAL contents via SQLite backup tooling). Current tables retain their layout; the old application ignores SQLx migration metadata. Revert application commit for code rollback; never delete or reset the database to recover. No existing user database is used during tests.

## Review and evidence

Independent review (gpt-6.1-sol): no blocking findings. Accepted two additional requirements: prove legacy initialization rollback after a forced failure, and concurrent initialization of the same legacy database. Acquire a SQLite immediate write transaction before inspecting schema; keep compatibility conversion and migration metadata transactional. Verify both indexes after dropping the renamed legacy table (index names survive rename). These requirements belong to chunks 1/2.

Baseline: `RUSTC_WRAPPER= cargo nextest run --test store` passed 8/8; `cargo search sqlx --limit 1` returned 0.9.0. Rust is 1.99.1. Initial normal commands failed because sccache returned Operation not permitted; a command-local empty RUSTC_WRAPPER fixes this without changing configuration. `git pull && git status` confirmed up to date and clean before edits.

Official references: https://github.com/transact-rs/sqlx and https://docs.rs/sqlx/latest/sqlx/sqlite/struct.SqliteConnectOptions.html .

Implementation agents: gpt-6.1-sol owned storage and tests; gpt-6-luna owned mechanical caller changes. A separate gpt-6.1-sol performed plan and implementation reviews. No agent edited another's owned files. Six new storage regression tests bring the focused suite from 8 to 14; existing live assertions are unchanged.

Live environment: `dev/models.sh up` initially failed because this host's `docker` is rootless Podman with SELinux, and container uid 1000 could not open cache lock files. A temporary `userns_mode: keep-id` attempt failed with crun's devpts mount error. A per-run Compose override using uid 0 inside the rootless namespace (mapped to the unprivileged host user) and `security_opt: [label=disable]` restored all four pinned servers to Healthy. No global configuration or committed Compose file changed. Temporary containers and network were removed after validation; temporary overrides/logs are removed before committing. Model cache and the local review image remain available.

Selected command output:

```text
$ cargo search sqlx --limit 1
sqlx = "0.9.0"
$ RUSTC_WRAPPER= cargo nextest run --test store
Summary [0.015s] 14 tests run: 14 passed, 0 skipped
$ RUSTC_WRAPPER= cargo clippy --all-targets -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.34s
$ RUSTC_WRAPPER= cargo nextest run
Summary [0.756s] 103 tests run: 103 passed, 22 skipped
$ RUSTC_WRAPPER= cargo nextest run --test live --run-ignored only --no-capture
Summary [25.423s] 22 tests run: 22 passed, 0 skipped
greeting holdout: precision=1, coverage=0.85, decided=17/20, correct=17
storage-risk holdout: precision=1, coverage=0.95, decided=19/20, correct=19
$ DOCKER_CONFIG=/tmp/8ball-sqlx/docker-config docker build -t 8ball:sqlx-review .
Finished `release` profile [optimized] target(s) in 2m 35s
Successfully tagged localhost/8ball:sqlx-review
bb3077ea13105e859c2c35955f2ac39ff4102eccae6085f0496544cc5ac5f4bf
$ docker run --rm 8ball:sqlx-review --version
8ball 0.1.0
$ docker image inspect 8ball:sqlx-review --format '{{.Config.User}}'
65532
```

No PostgreSQL implementation, performance benchmark, external source-code scan, remote push or deployment was performed. The existing v1 report describes its original commit; this document records validation of this migration.
