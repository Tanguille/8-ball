# 8-ball conventions

Follow [docs/rust-style.md](docs/rust-style.md). Write docs in the Plain mode of [SimpleEnglish](https://github.com/AminBlg/SimpleEnglish), and keep them short.

- Write commits in Commitizen style, with no AI attribution. Stage files by path.
- Before you say that work is done, run `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings`, and `cargo nextest run`.
- For the live tests, run `dev/models.sh up`, then `cargo nextest run --test live --run-ignored only --no-capture`.
- Never loosen a live assertion without asking.
