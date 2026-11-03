# Rust Code Style

Adapted from Mesh Organiser’s `docs/rust-style.md` (last changed in `3c28dc4d`) on 2026-11-03. Applies to production code and tests. 8-ball-specific conventions take precedence where noted.

## General Conventions

- Follow standard Rust conventions (rustfmt auto-formats)
- Use sqlx for database queries
- Enable clippy pedantic, nursery and style lints; run checks with `-D warnings`.
- In `format!` / `format_args!`, prefer **inline names in braces** (`{var}`) when the value is already in a variable, so the string is self-explanatory. Example: `format!("IN ({placeholders})")` rather than `format!("IN ({})", placeholders)`. Do not introduce a variable only to use it in braces; keep the argument form when the value is an expression (e.g. `format!("IN ({})", join(ids, ","))`).

### Identifiers

- Prefer **full words** (`model_id`, `total`, `user`, `chunk`) over single-letter names.
- **Exception:** `e` for an error value in `if let Err(e)`, `Result::map_err(|e| ...)`, and similar is fine.
- **Narrow exception:** trivial **loop indices** in very short loops (`for i in …`, `enumerate` with `i` / `j`) when conventional; still prefer a word if the index carries meaning beyond position.
- Avoid one-letter names for domain data (`n`, `x`, `t`, `m`) unless a longer name would be genuinely unwieldy (rare).

## Control flow and whitespace

- Prefer a **blank line** between preceding logic and a **`return`** (including `return Ok(...)`, `return Err(...)`, or early exit from a branch), so the exit path is easy to scan. Apply the same spacing before the **final expression** in a function when it follows a multi-line or non-trivial block (even if you omit the `return` keyword).
- Omit the extra line when it would hurt readability (e.g. a one-line guard `if !ready { return; }` at the top of a function, or a very short `match` arm).

**Good:**

```rust
fn sum_ids(ids: &[u64]) -> u64 {
    let mut total = 0;
    for &id in ids {
        total += id;
    }

    total
}
```

## Types and invariants (“tight typing”)

Prefer types that match the **domain**: if a value cannot meaningfully be negative, prefer an **unsigned** integer (`u32`, `u64`, `usize`) or a **newtype** / wrapper that enforces validation at construction.

- **Counts, lengths, sizes, non-negative offsets**: prefer `usize` for indexing and in-memory sizes; `u32`/`u64` for stable wire or storage sizes when the domain is non-negative.
- **Signed types** (`i32`, `i64`): use when the domain includes negative values or API/schema compatibility requires them (e.g. SQLite `INTEGER`) — document why if it looks “loose”. Represent nullable foreign keys with `Option`, not a negative sentinel.
- **Boundaries**: external formats (DB columns, HTTP query params, serde) often fix the type; validate at the edge and convert explicitly rather than scattering `as u64` / silent casts. Watch for `clippy::cast_possible_wrap` / sign-loss when mixing signed and unsigned.

For critical ranges (e.g. “must be 1..=100”), consider a **small type with a constructor** (`fn new(x: i32) -> Option<Self>`) so call sites cannot hold an invalid value ([Rust book: custom types for validation](https://doc.rust-lang.org/book/ch09-03-to-panic-or-not-to-panic.html#custom-types-for-validation)).

**Existing code:** tighten types in **new** code and at **clear boundaries** (new endpoints, DB mappers). Avoid unrelated mass migrations of `i64`/`i32` across the codebase in a single change set.

## Strings

Rust’s main UTF-8 text types are **`String`** (owned), **`&str`** (borrowed), and **`Cow<'_, str>`** when you may borrow or allocate. See the [Rust book on strings](https://doc.rust-lang.org/book/ch08-02-strings.html) for indexing and UTF-8 pitfalls.

### Choosing parameter types

| Situation                                                                                                            | Prefer                                                                                                                                                       |
| -------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Read-only, borrowed text in **most** internal helpers                                                                | **`&str`** — simplest; `&String` and `&str` both coerce for callers ([`Deref`](https://doc.rust-lang.org/std/ops/trait.Deref.html) to `str`).                |
| **Public / shared** API that should accept `String`, `&str`, `Cow<str>`, etc. without awkward `&` at every call site | **`impl AsRef<str>`** — call `s.as_ref()` once in the body. See [`AsRef`](https://doc.rust-lang.org/std/convert/trait.AsRef.html).                           |
| Multiple string parameters each needing `AsRef<str>`                                                                 | Separate type parameters, e.g. `S1: AsRef<str>, S2: AsRef<str>`, or use `&str` if callers can pass `&` — otherwise the compiler may unify one type for both. |
| **Own** or **consume** the text (store, mutate, move on)                                                             | **`String`** or **`impl Into<String>`** (accepts `String` and `&str` with allocation cost for the latter).                                                   |
| Sometimes borrow, sometimes allocate (e.g. optional default)                                                         | **`Cow<'_, str>`**.                                                                                                                                          |

**`impl AsRef<str>` is not the default everywhere:** it adds generic noise and monomorphization; many codebases use **`&str`** for most functions and reserve **`AsRef<str>`** for crate boundaries or helpers called with mixed owned/borrowed strings. Use whichever matches call sites without extra ceremony.

### Paths

Use **`&Path`**, **`PathBuf`**, or **`impl AsRef<Path>`** for filesystem paths — not `AsRef<str>` (paths are not always valid UTF-8 on all platforms). See [`std::path::Path`](https://doc.rust-lang.org/std/path/struct.Path.html).

## Imports

Where multiple items are imported from the same path, use a single `use` statement with a braced list instead of multiple separate lines (one `use` per path prefix).

**Good:**

```rust
use std::path::{Path, PathBuf};
use service::{AppState, Configuration};
```

**Avoid:**

```rust
use std::path::Path;
use std::path::PathBuf;
use service::AppState;
use service::Configuration;
```

`cargo fmt` does not merge imports; this is a project convention. For very long braced lists (e.g. 10+ items), split by logical subpath for readability.

### What to import

Bring in the **module** for free functions, and the **type** itself for types:

```rust
use std::fs;                 // fs::read_dir(..), fs::write(..)
use std::time::Duration;     // Duration::from_secs(5)
```

### Import only what you use more than once

Import a path only when the name appears **more than once** in the file. A path used a single time stays **fully qualified at the call site** — an import costs a line at the top plus a lookup for the reader, and only pays for itself when the name is reused.

**Good:**

```rust
use std::fs;

let config = fs::read_to_string(&config_path)?;
let cache = fs::read_to_string(&cache_path)?;

// used once, so no import
let db_path = std::path::Path::new("model.sqlite");
```

**Avoid:**

```rust
use std::path::Path;

let db_path = Path::new("model.sqlite"); // the only use in the file
```

### No renamed imports

Do not rename with `as` in a `use`. When two items share a name, qualify one at the **use site** instead:

**Good:**

```rust
use std::fs;
use tokio::fs::File;

let handle = File::create(&path).await?;      // tokio
let meta = fs::File::create(&meta_path)?;     // std, qualified by its module
```

**Avoid:**

```rust
use std::fs::{self, File as StdFile};
use tokio::fs::File;
```

The single exception is `use some::Trait as _;`, which imports a trait for method resolution without binding a name. That is not a rename, and it cannot be written at the call site. Traits required for method resolution may be imported even when used only once.

### Placement

All `use` statements belong at the **top of the file**. Do not add or widen a `use` inside a nested module or function body just to shorten a path — if a nested scope (a `#[cfg(test)] mod tests`, an inner `mod get`) would need its own import, leave the path fully qualified there.

### Order

List `use` statements in this order (top to bottom), with a single blank line between groups. Omit any group that does not appear in the file.

1. **std** – standard library (`use std::...`)
2. **External** – third-party / crates.io crates (e.g. `serde`, `tokio`, `axum`)
3. **Crates** – the `eightball` library in the binary and integration tests, or other workspace member crates if introduced
4. **crate** – current crate (`use crate::...`)
5. **super** – parent module (`use super::...`)

Example (one `use` per path, single `use crate::...` with braced list):

```rust
use std::path::PathBuf;

use serde::Serialize;
use sqlx::SqlitePool;

use eightball::config::Config;

use crate::{backend::Endpoint, service::Service};

use super::BackendError;
```

### Module declarations

Follow the [Rust style guide](https://doc.rust-lang.org/beta/style-guide/items.html): `use` statements and module declarations (`mod foo;`) must come before other items. Put **imports before module declarations**.

- In each file: first all `use` statements (in the [import order](#order) above), then all `mod` declarations, then the rest of the file (types, functions, re-exports, etc.).
- Sort module declarations **alphabetically** (version-sort). Keep `#[cfg(...)]` attached to the `mod` it applies to, with layout determined by rustfmt; sort only by the module name when comparing.
- **Feature-gated items** follow the same order: cfg-gated `use` in the use section, cfg-gated `mod` in the mod section, cfg-gated `pub use` or code in the rest. Do not group by feature at the expense of use-before-mod.
- Do not move or reorder `#[macro_use]` or other attribute-annotated mods if that could change semantics.

### Module file layout

Prefer the **module-name-as-filename** layout over top-level `mod.rs` files ([Rust book](https://doc.rust-lang.org/book/ch07-05-separating-modules-into-different-files.html)):

- For a module `foo`, use **`foo.rs`** (same directory as the parent), not **`foo/mod.rs`**.
- Submodules of `foo` live in the **`foo/`** directory (e.g. `foo/bar.rs` when `foo` contains `mod bar;`). The compiler looks up child modules there, not as siblings of `foo.rs`.

This avoids many files named `mod.rs` and keeps the module tree easier to follow. 8-ball always uses this layout; do not introduce `mod.rs`.

Example:

- **Prefer:** `src/api.rs` and `src/api/blob_api.rs`, `src/api/model_api.rs`, …
- **Avoid:** `src/api/mod.rs` and `src/api/blob_api.rs`, … (same children, but parent is the redundant `mod.rs`).

## Error Handling

Preserve the existing module-specific `thiserror` error types and use `?` for propagation. Use `anyhow` only in `main.rs`. Do not add a generic application error wrapper for style consistency.

Keep SQLite row ids as `i64` because they mirror SQLite INTEGER values. Preserve existing wire types, serialized field names, error messages, and public API names during style-only changes.

## 8-ball conventions

- Rust 2024 with the stable toolchain.
- Comments explain non-obvious constraints, not syntax.
- Tracing writes to stderr; stdout carries JSON results and MCP JSON-RPC only.
- Verify crate APIs in the local Cargo registry source before using them.
- Behavior changes require failing regression tests first. Style-only edits use existing tests to prove behavior is unchanged; do not add tests for whitespace or local names.
- Never weaken live-test assertions to accommodate a refactor.

## Commands

Run from the repository root:

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo nextest run
# When live verification is required:
dev/models.sh up
cargo nextest run --test live --run-ignored only --no-capture
```

Prefer explicit conversions (`try_from`, `checked_*`, `unsigned_abs`) at domain boundaries. Preserve existing types during style-only changes; introduce no lint suppressions merely to satisfy the style guide.
