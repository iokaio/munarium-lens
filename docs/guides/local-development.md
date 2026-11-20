# Run the CPU scaffold

Install [rustup](https://rustup.rs/) and run these commands from the checkout.
Rustup uses the pinned toolchain from `rust-toolchain.toml`. The first build
can fetch compiler components and crates. Once cached, the test runs offline
without model weights, GPU, credentials or Server.

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --offline
cargo build --locked --offline
cargo run --locked --offline
```

In a second terminal, use the executable `curl` (`curl.exe` on Windows).

```sh
curl -i http://127.0.0.1:8787/healthz
curl -i http://127.0.0.1:8787/readyz
curl -i -H "Content-Type: application/json" --data-binary @tests/fixtures/embed.json http://127.0.0.1:8787/v1/embeddings
curl -i -H "Content-Type: application/json" --data-binary @tests/fixtures/rerank.json http://127.0.0.1:8787/v1/rerank
```

Liveness returns HTTP 204. Readiness returns HTTP 503 with
`{"code":"backend_not_ready"}`. Each inference call returns HTTP 503 with
`{"code":"inference_unavailable","stage":"embedding"}` or
`{"code":"inference_unavailable","stage":"reranking"}`. Requests over
16 KiB receive HTTP 413. Stop the process with Ctrl+C. If 8787 is already in
use, set `LENS_PORT` for the current shell and use the port reported at startup.

Run the contributor checks in [Contributing](../../CONTRIBUTING.md) before
opening a pull request. See the [serving guide](../ops/serving.md) for
the proposed backend lifecycle.
