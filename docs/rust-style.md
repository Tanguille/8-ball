# Rust Code Style

This guide applies to production code and tests. 8-ball conventions take precedence where noted.

## General Conventions

- Follow standard Rust conventions. rustfmt formats the code automatically.
- Use sqlx for database queries.
- Enable clippy pedantic, nursery, and style lints. Run clippy with `-D warnings`.
- If a value is in a variable, prefer an inline name (`{var}`) in `format!` and `format_args!`. The string then explains itself. For example, write `format!("IN ({placeholders})")`, not `format!("IN ({})", placeholders)`.
- Do not add a variable only to use it in braces.
- If the value is an expression, keep the argument form, for example `format!("IN ({})", join(ids, ","))`.

### Identifiers

- Prefer full words (`model_id`, `total`, `user`, `chunk`) over single-letter names.
- You can use `e` for an error value in `if let Err(e)`, `Result::map_err(|e| ...)`, and similar code.
- In very short loops, you can use conventional loop indices (`for i in …`, `enumerate` with `i` / `j`). If the index means more than position, prefer a word.
- Avoid one-letter names for domain data (`n`, `x`, `t`, `m`). The rare exception is a longer name that is unwieldy.

## Control flow and whitespace

- Prefer a blank line between preceding logic and a `return`, so that the exit path is easy to scan. This includes `return Ok(...)`, `return Err(...)`, and an early exit from a branch.
- If the final expression of a function follows a multi-line or non-trivial block, put the same blank line before it. This rule applies with or without the `return` keyword.
- If the extra line hurts readability, omit it. Examples are a one-line guard `if !ready { return; }` at the top of a function and a very short `match` arm.

Good:

```rust
fn sum_ids(ids: &[u64]) -> u64 {
    let mut total = 0;
    for &id in ids {
        total += id;
    }

    total
}
```

## Types and invariants ("tight typing")

Prefer types that match the domain. If a value cannot meaningfully be negative, prefer an unsigned integer (`u32`, `u64`, `usize`). A newtype or wrapper that enforces validation at construction is equally good.

- Counts, lengths, sizes, and non-negative offsets: Prefer `usize` for indexing and in-memory sizes. For stable wire or storage sizes in a non-negative domain, prefer `u32` or `u64`.
- Signed types (`i32`, `i64`): If the domain includes negative values, or if API or schema compatibility requires them (as SQLite `INTEGER` does), use them. If the type looks loose, document why. Represent nullable foreign keys with `Option`, not a negative sentinel.
- Boundaries: External formats (database columns, HTTP query parameters, serde) often fix the type. Make sure that values are valid at the edge. Convert explicitly, and do not scatter `as u64` or silent casts. Prefer explicit conversions (`try_from`, `checked_*`, `unsigned_abs`) at domain boundaries. If you mix signed and unsigned types, watch for `clippy::cast_possible_wrap` and sign loss.

For critical ranges (for example, "must be 1..=100"), consider a small type with a constructor (`fn new(x: i32) -> Option<Self>`). Call sites then cannot hold an invalid value. See [Rust book: custom types for validation](https://doc.rust-lang.org/book/ch09-03-to-panic-or-not-to-panic.html#custom-types-for-validation).

In existing code, tighten types in new code and at clear boundaries (new endpoints, database mappers). Avoid unrelated mass migrations of `i64` or `i32` across the codebase in one change set.

## Imports

If you import multiple items from the same path, use one `use` statement with a braced list. Use one `use` per path prefix.

Good:

```rust
use std::path::{Path, PathBuf};
use service::{AppState, Configuration};
```

Avoid:

```rust
use std::path::Path;
use std::path::PathBuf;
use service::AppState;
use service::Configuration;
```

`cargo fmt` does not merge imports. This is a project convention. If a braced list is very long (for example, 10 or more items), split it by logical subpath.

### What to import

Import the module for free functions. Import the type itself for types:

```rust
use std::fs;                 // fs::read_dir(..), fs::write(..)
use std::time::Duration;     // Duration::from_secs(5)
```

### Import only what you use more than once

If a name appears more than once in the file, import its path. If you use a path one time, keep it fully qualified at the call site. An import costs a line at the top and a lookup for the reader. Only a reused name pays for that cost.

Good:

```rust
use std::fs;

let config = fs::read_to_string(&config_path)?;
let cache = fs::read_to_string(&cache_path)?;

// used once, so no import
let db_path = std::path::Path::new("model.sqlite");
```

Avoid:

```rust
use std::path::Path;

let db_path = Path::new("model.sqlite"); // the only use in the file
```

### No renamed imports

Do not rename with `as` in a `use`. If two items share a name, qualify one at the use site:

Good:

```rust
use std::fs;
use tokio::fs::File;

let handle = File::create(&path).await?;      // tokio
let meta = fs::File::create(&meta_path)?;     // std, qualified by its module
```

Avoid:

```rust
use std::fs::{self, File as StdFile};
use tokio::fs::File;
```

The only exception is `use some::Trait as _;`, which imports a trait for method resolution without binding a name. That is not a rename, and you cannot write it at the call site. You can import a trait that method resolution requires, and one use is enough.

### Placement

Put all `use` statements at the top of the file. Do not add or widen a `use` inside a nested module or function body to shorten a path. If a nested scope (`#[cfg(test)] mod tests`, inner `mod get`) needs its own import, keep the path fully qualified there.

### Order

List `use` statements in this order, from top to bottom. Put one blank line between groups. Omit any group that does not appear in the file.

1. std: The standard library (`use std::...`)
2. External: Third-party crates.io crates (for example, `serde`, `tokio`, `axum`)
3. Crates: The workspace members `domain`, `store`, `backend`, and `service`
4. crate: The current crate (`use crate::...`)
5. super: The parent module (`use super::...`)

This example has one `use` per path and one `use crate::...` with a braced list:

```rust
use std::path::PathBuf;

use serde::Serialize;
use sqlx::SqlitePool;

use domain::config::Config;

use crate::{evaluate::Report, metrics::Metrics};

use super::BackendError;
```

### Module declarations

Follow the [Rust style guide](https://doc.rust-lang.org/beta/style-guide/items.html). `use` statements and module declarations (`mod foo;`) must come before other items. Put imports before module declarations.

- In each file, put all `use` statements first, in the [import order](#order) above. Then put all `mod` declarations. Then put the rest of the file (types, functions, re-exports, and other items).
- Sort module declarations alphabetically (version-sort). Keep `#[cfg(...)]` attached to the `mod` that it applies to. rustfmt determines the layout. Sort only by the module name.

### Module file layout

Prefer the module-name-as-filename layout over top-level `mod.rs` files ([Rust book](https://doc.rust-lang.org/book/ch07-05-separating-modules-into-different-files.html)). For a module `foo`, use `foo.rs` in the same directory as the parent, not `foo/mod.rs`. Put the submodules of `foo` in the `foo/` directory. For example, `mod bar;` in `foo` uses `foo/bar.rs`. The compiler looks up child modules there, not as siblings of `foo.rs`.

This layout prevents many files named `mod.rs` and keeps the module tree easy to follow. 8-ball always uses this layout. Do not add `mod.rs`.

For example, prefer `src/parser.rs` with `src/parser/lexer.rs` and `src/parser/tokens.rs`. Avoid `src/parser/mod.rs` with the same children, because the parent is then the redundant `mod.rs`. A crate root is `src/lib.rs` or `src/main.rs`, so `backend/src/lib.rs` has `backend/src/embed.rs` beside it.

## Error Handling

Keep the existing module-specific `thiserror` error types. Use `?` for propagation. Use `anyhow` only in `main.rs`. Do not add a generic application error wrapper for style consistency.

Keep SQLite row ids as `i64`, because they mirror SQLite INTEGER values. During style-only changes, keep existing types, wire types, serialized field names, error messages, and public API names.

## 8-ball conventions

- Use Rust 2024 with the stable toolchain.
- Write comments for non-obvious constraints, not syntax.
- Tracing writes to stderr. stdout carries JSON results and MCP JSON-RPC only.
- Ids are SQLite INTEGER rowids, not UUIDs, because people type them by hand, as in `8ball label 42 yes`.
- Before you use a crate API, make sure that it matches the local Cargo registry source.
- For a behavior change, write a failing regression test first.
- For a style-only edit, use existing tests to prove that behavior is unchanged. Do not add tests for whitespace or local names.
- Do not add lint suppressions only to satisfy this guide.
