# Rust code style

This guide applies to production code and tests. 8-ball conventions take precedence where noted.

## General conventions

- Follow standard Rust conventions. rustfmt formats the code automatically.
- Use sqlx for database queries.
- Enable clippy pedantic, nursery, and style lints. Run clippy with `-D warnings`.
- If the value already has a name, use it in the format string with `{var}`. This applies to `format!` and `format_args!`. For example, write `format!("IN ({placeholders})")`.
- Keep a value in argument form if using braces would require a new variable.
- Keep expressions as arguments. For example, write `format!("IN ({})", join(ids, ","))`.

### Identifiers

- Prefer full words (`model_id`, `total`, `user`, `chunk`) over single-letter names.
- You can use `e` for an error value in `if let Err(e)`, `Result::map_err(|e| ...)`, and similar code.
- In very short loops, you can use standard index names (`for i in …`, `enumerate` with `i` / `j`). If the index has a meaning beyond position, use a word.
- Use words for domain data. Use a single letter such as `n`, `x`, `t`, or `m` only if a longer name is hard to read.

## Control flow and whitespace

- Prefer a blank line before a `return` so that readers can spot where the code exits. This includes `return Ok(...)`, `return Err(...)`, and early exits from branches.
- If the final expression follows a block with multiple lines or steps, put a blank line before it. This also applies to implicit returns.
- Skip the blank line where it makes the code harder to read. Examples include a short guard such as `if !ready { return; }` and a short `match` arm.

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

Choose a type that fits the valid values. If a value cannot be negative, prefer an unsigned integer (`u32`, `u64`, `usize`). You can also use a wrapper type that checks the value when you create it.

- For counts, lengths, sizes, and offsets in memory, prefer `usize` when the value is non-negative. For non-negative values in a stable API or storage format, prefer `u32` or `u64`.
- Use signed types (`i32`, `i64`) when negative values are valid or when the API or schema requires them. SQLite `INTEGER` is one example. If the reason for a signed type is unclear, document it. For a foreign key that can be missing, use `Option`.
- External formats often fix the type, as with database columns, HTTP query parameters, and serde. Make sure that values are valid at that boundary. Use explicit conversions such as `try_from`, `checked_*`, and `unsigned_abs`. Avoid scattered casts such as `as u64` and casts that silently change the value. If you mix signed and unsigned types, watch for `clippy::cast_possible_wrap` and sign loss.

For an important range such as "must be 1..=100", consider a type that enforces it. Its constructor (`fn new(x: i32) -> Option<Self>`) can reject invalid values. See the [Rust book on custom types for validation](https://doc.rust-lang.org/book/ch09-03-to-panic-or-not-to-panic.html#custom-types-for-validation).

Use more precise types in new code and at clear boundaries such as new endpoints or database mappers. Keep unrelated changes to `i64` or `i32` out of the same change.

## Imports

If multiple imports share a path prefix, group them in one `use` statement with braces.

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

Group imports by hand because `cargo fmt` leaves them separate. If a list grows long, for example to 10 or more items, split it by subpath.

### What to import

For free functions, import their module. For types, import the type itself.

```rust
use std::fs;                 // fs::read_dir(..), fs::write(..)
use std::time::Duration;     // Duration::from_secs(5)
```

### Import only what you use more than once

If you use a name more than once in a file, import it. If you use it once, write out the full path where you use it. This keeps the path visible where the reader needs it.

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

Use the original name in a `use` statement. If two items share a name, write out the path for one where you use it.

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

If a method needs a trait in scope, you can use `use some::Trait as _;`. This makes the trait's methods available without adding a name. One use of the method is enough to allow this import.

### Placement

Put all `use` statements at the top of the file. If a nested scope such as `#[cfg(test)] mod tests` or inner `mod get` needs its own import, write out the path there. This also applies to function bodies.

### Order

List `use` statements in this order, from top to bottom. Put one blank line between groups. Omit any group that does not appear in the file.

1. std: The standard library (`use std::...`)
2. External: Third-party crates.io crates (for example, `serde`, `tokio`, `axum`)
3. Crates: The workspace members `domain`, `store`, `backend`, and `service`
4. crate: The current crate (`use crate::...`)
5. super: The parent module (`use super::...`)

The example uses one `use` per path. The `use crate::...` statement groups its imports in braces.

```rust
use std::path::PathBuf;

use serde::Serialize;
use sqlx::SqlitePool;

use domain::config::Config;

use crate::{evaluate::Report, metrics::Metrics};

use super::BackendError;
```

### Module declarations

Follow the [Rust style guide](https://doc.rust-lang.org/beta/style-guide/items.html). Put imports first, then module declarations (`mod foo;`), then the rest of the file.

Use the [import order](#order) above. Sort module declarations by name in version-sort order. Keep `#[cfg(...)]` with the `mod` that it applies to. Let rustfmt set the layout.

### Module file layout

Use the name of each module as its file name. For a module `foo`, use `foo.rs` next to the parent file. Put the child modules of `foo` in `foo/`. For example, if `foo` declares `mod bar;`, the compiler looks for `foo/bar.rs`. See the [Rust book on module files](https://doc.rust-lang.org/book/ch07-05-separating-modules-into-different-files.html).

This layout gives each module a distinct file name. 8-ball uses it throughout the project. Do not add `mod.rs`.

For example, use `src/parser.rs` with `src/parser/lexer.rs` and `src/parser/tokens.rs`. For a crate root, use `src/lib.rs` or `src/main.rs`. The module file `backend/src/embed.rs` sits next to its crate root, `backend/src/lib.rs`.

## Error handling

Keep the existing `thiserror` types for each module. Use `?` to pass an error back to the caller. Use `anyhow` only in `main.rs`. Keep module-specific types when you update code style.

Keep SQLite row ids as `i64`, because they mirror SQLite INTEGER values. During style-only changes, keep existing types, wire types, serialized field names, error messages, and public API names.

## 8-ball conventions

- Use Rust 2024 with the stable toolchain.
- Use comments to explain constraints that the code leaves unclear.
- Send tracing to stderr. Keep stdout for JSON results and MCP JSON-RPC.
- Use SQLite INTEGER rowids because people type ids by hand, as in `8ball label 42 yes`.
- Before you use a crate API, make sure that it matches the local Cargo registry source.
- For a behavior change, write a failing regression test first.
- For a style-only edit, use existing tests to prove that behavior is unchanged. Do not add tests for whitespace or local names.
- Do not add lint suppressions only to satisfy this guide.
