# Run Lens locally

Install [rustup](https://rustup.rs/) and run these commands from the checkout.
Rustup uses the pinned toolchain from `rust-toolchain.toml`. The first build
can fetch compiler components and crates. Once cached, the tests run offline
without model weights, GPU, credentials or Server.

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --offline
cargo build --locked --offline
```

## Default: both stages disabled

`cargo run --locked --offline` starts on `127.0.0.1:8787` with no backend.
In a second terminal, use the executable `curl` (`curl.exe` on Windows).

```sh
curl -i http://127.0.0.1:8787/healthz
curl -i http://127.0.0.1:8787/readyz
curl -i http://127.0.0.1:8787/v1/models
curl -i -H "Content-Type: application/json" --data-binary @tests/fixtures/embed.json http://127.0.0.1:8787/v1/embeddings
```

Liveness returns HTTP 204. Readiness returns HTTP 503 with
`{"code":"backend_not_ready"}` and discovery returns an empty list. Each
inference call returns HTTP 503 with `{"code":"inference_unavailable","stage":"embedding"}`
or the `reranking` equivalent. Stop the process with Ctrl+C.

## Reference backend

The reference backend is deterministic and **not a retrieval model**. Use it
for tests and protocol checks only. Set the variables for the current shell
(PowerShell: `$env:LENS_EMBEDDING_BACKEND = "reference"`), then run Lens:

```sh
LENS_EMBEDDING_BACKEND=reference LENS_RERANKING_BACKEND=reference cargo run --locked --offline
curl -s http://127.0.0.1:8787/readyz
curl -s http://127.0.0.1:8787/v1/models
curl -s -H "Content-Type: application/json" --data-binary @tests/fixtures/reference-embed.json http://127.0.0.1:8787/v1/embeddings
curl -s -H "Content-Type: application/json" --data-binary @tests/fixtures/reference-rerank.json http://127.0.0.1:8787/v1/rerank
```

Readiness returns HTTP 200 with each stage's served model, `"backend":"reference"`,
`"retrieval_model":false`, the embedding dimension and the request limits.
The rerank fixture returns `{"model":"lens-reference/rerank-v1","results":[{"index":1,"relevance_score":0.6},{"index":0,"relevance_score":0.0}],"usage":{"prompt_tokens":18,"total_tokens":18}}`.
The [endpoint profile](../design/endpoint-profile.md#lens-1-implemented-profile-clarifications-proposed)
lists every refusal code and the order of checks.

| Variable | Default | Meaning |
|---|---|---|
| `LENS_HOST`, `LENS_PORT` | `127.0.0.1`, `8787` | Listener address. A non-loopback host also needs the next variable. |
| `LENS_ALLOW_UNAUTHENTICATED_NON_LOOPBACK` | `0` | `1` permits a non-loopback listener. Lens has no service authentication yet; use it only inside an isolated container network. |
| `LENS_EMBEDDING_BACKEND`, `LENS_RERANKING_BACKEND` | `disabled` | `disabled` or `reference`. |
| `LENS_REFERENCE_DIMENSIONS` | `1024` | Reference embedding dimension, 1–4096. Part of the served name. |
| `LENS_MAX_BODY_BYTES` | `262144` | Largest request body. |
| `LENS_MAX_ITEMS` | `64` | Most embedding inputs or rerank documents per request. |
| `LENS_MAX_INPUT_TOKENS` | `512` | Most tokens per input or per query and document pair. |
| `LENS_MAX_BATCH_TOKENS` | `16384` | Most tokens per request. |
| `LENS_MAX_IN_FLIGHT`, `LENS_MAX_QUEUE`, `LENS_QUEUE_TIMEOUT_MS` | `4`, `16`, `1000` | Per-stage execution slots, queue places and queue wait before `overloaded`. |

The defaults are placeholders for local checks, not measured capacity. An
invalid value stops startup with a message naming the variable.

## Reference container

The [Dockerfile](../../Dockerfile) builds a static binary on a distroless base
with the reference backend only. Publish the port on the host loopback
interface and enable the container listener explicitly:

```sh
docker build -t munarium-lens-reference:local .
docker run --rm -p 127.0.0.1:8787:8787 -e LENS_HOST=0.0.0.0 -e LENS_ALLOW_UNAUTHENTICATED_NON_LOOPBACK=1 -e LENS_EMBEDDING_BACKEND=reference -e LENS_RERANKING_BACKEND=reference munarium-lens-reference:local
```

Without those variables the container listens on its own loopback interface
and reports no ready stage. `docker stop` sends SIGTERM and Lens finishes
accepted requests before it exits.

Run the contributor checks in [Contributing](../../CONTRIBUTING.md) before
opening a pull request. See the [serving guide](../ops/serving.md) for
the proposed production lifecycle.
