# Development

The code is a Cargo workspace. `domain` holds questions, configuration, and verdict math. `store` is the SQLite storage, `backend` calls the model servers, `service` is the logic with its REST and MCP routes, and `cli` is the `8ball` binary.

Follow the [Rust style guide](rust-style.md) for source and tests. Write commits in Commitizen style, and write docs in the Plain mode of [SimpleEnglish](https://github.com/AminBlg/SimpleEnglish). Do not loosen a live test assertion to make it pass.

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo nextest run
dev/models.sh up
cargo nextest run --test live --run-ignored only --no-capture
docker build -t 8ball:dev .
bash dev/image-smoke.sh 8ball:dev
claude plugin validate plugin --strict
```

This repository claims no public marketplace submission. A hosted privacy-policy URL for directory publication is deferred.

The live tests need `dev/models.sh up`. This script takes `up`, `bench`, `down`, `status` (the default), or `logs [service]`. It runs `d3`, `embed`, `rerank`, and `chat` from `dev/compose.yaml` on ports 9931 to 9934. `bench` also starts the [benchmark](benchmark.md) baseline `big` on port 9935. They use the same pinned llama.cpp image and models as the deployment. `dev/config.toml` is the matching configuration. `dev/image-smoke.sh <image>` tests the version, startup, SQLite initialization, authentication, and triage with a read-only root filesystem.

