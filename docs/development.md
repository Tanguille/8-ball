# Development

The code is a Cargo workspace.

- `domain` defines questions, configuration, and how to turn model scores into answers.
- `store` reads from and writes to SQLite.
- `backend` calls the model servers.
- `service` handles requests and exposes the REST and MCP routes.
- `cli` provides the `8ball` command.
- `bench` runs the [benchmark](benchmark.md).

Follow the [Rust style guide](rust-style.md) for source and tests. Write commits in Commitizen style. Write docs in the Plain mode of [SimpleEnglish](https://github.com/AminBlg/SimpleEnglish).

## Run the local checks

Run these commands from the repository root.

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo nextest run
```

## Run the live tests

Start the models before you run live tests. Do not loosen a live test assertion to make it pass.

```sh
dev/models.sh up
cargo nextest run --test live --run-ignored only --no-capture
```

The script accepts `up`, `bench`, `down`, `status` (the default), and `logs [service]`. The services in `dev/compose.yaml` run `d3`, `embed`, `rerank`, and `chat` on ports 9931 to 9934. Use `dev/config.toml` with these services. The `bench` command also starts `big`, the [benchmark](benchmark.md) baseline, on port 9935. These services use the same pinned llama.cpp image and models as the deployment.

## Test the container and plugin

The stack works with Docker and rootless Podman, including with SELinux enforcing.

```sh
docker build -t 8ball:dev .
dev/image-smoke.sh 8ball:dev
claude plugin validate plugin --strict
```

The `dev/image-smoke.sh <image>` script tests the version, startup, SQLite setup, authentication, and triage with a read-only root filesystem.
