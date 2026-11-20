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

To check the feature graphs as well, repeat the clippy and test commands with
`--features fetch`, `--features qwen3` and `--features qwen3,fetch`. The
fetcher's tests use a local in-process HTTP server on `127.0.0.1` and never
contact Hugging Face or GitHub. With [cargo-deny](https://github.com/EmbarkStudios/cargo-deny)
installed, `cargo deny check` and `cargo deny --all-features check` apply the
license, advisory, ban and source policy in [`deny.toml`](../../deny.toml),
as the advisory supply-chain workflow does.

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
| `LENS_EMBEDDING_BACKEND`, `LENS_RERANKING_BACKEND` | `disabled` | `disabled` or `reference`, and `qwen3` in a build with that feature. |
| `LENS_EMBEDDING_ARTIFACTS`, `LENS_RERANKING_ARTIFACTS` | unset | Artifact directory of a `qwen3` stage. |
| `LENS_EMBEDDING_MANIFEST_SHA256`, `LENS_RERANKING_MANIFEST_SHA256` | unset | Expected SHA-256 of that directory's `lens-manifest.json`. |
| `LENS_EMBEDDING_BUNDLE`, `LENS_RERANKING_BUNDLE` | unset | `fetch` builds: bundle fetched for a `qwen3` stage at startup. Replaces `LENS_*_ARTIFACTS`. |
| `LENS_EMBEDDING_BUNDLE_REVISION`, `LENS_RERANKING_BUNDLE_REVISION` | unset | Pinned Hugging Face commit of that bundle, 40 lowercase hex characters. |
| `LENS_MODEL_CACHE_DIR` | unset | Writable directory that fetched bundles go into, outside any Lens checkout. Required with a bundle. |
| `LENS_MODEL_FETCH_TIMEOUT_SECS` | `3600` | Total time allowed to download one file, 1–86400. Only with a bundle. |
| `LENS_REFERENCE_DIMENSIONS` | `1024` | Reference embedding dimension, 1–4096. Part of the served name. |
| `LENS_MAX_BODY_BYTES` | `262144` | Largest request body. |
| `LENS_MAX_ITEMS` | `64` | Most embedding inputs or rerank documents per request. |
| `LENS_MAX_INPUT_TOKENS` | `512` | Most tokens per input or per query and document pair. |
| `LENS_MAX_BATCH_TOKENS` | `16384` | Most tokens per request. |
| `LENS_MAX_IN_FLIGHT`, `LENS_MAX_QUEUE`, `LENS_QUEUE_TIMEOUT_MS` | `4`, `16`, `1000` | Per-stage execution slots, queue places and queue wait before `overloaded`. |

The defaults are placeholders for local checks, not measured capacity. An
invalid value stops startup with a message naming the variable.

## Qwen3 backend (optional feature)

The `qwen3` feature adds Qwen3-Embedding-0.6B and Qwen3-Reranker-0.6B on
Candle, CPU only. It is off by default and is not qualified; see the
[backend design](../design/qwen3-backend.md). A `qwen3` build alone never
downloads anything. Adding the `fetch` feature lets Lens download a
published, pinned bundle from the fixed Munarium locations (see
[Fetching a published bundle](#fetching-a-published-bundle)); the steps here
use the publisher's files instead. Obtain the publisher's `config.json`, `tokenizer.json` and
`model.safetensors` for each model at the pinned revisions yourself, into a
directory outside the repository or under the ignored `models/`. Check their
SHA-256 values against the publisher's, then write a manifest for each and
note the printed digest:

```sh
python scripts/lens_manifest.py --stage embedding --served-model qwen3-embedding-0.6b-f32-1024d --repository Qwen/Qwen3-Embedding-0.6B --revision 97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3 models/qwen3-embedding-0.6b
python scripts/lens_manifest.py --stage reranking --served-model qwen3-reranker-0.6b-f32 --repository Qwen/Qwen3-Reranker-0.6B --revision e61197ed45024b0ed8a2d74b80b4d909f1255473 models/qwen3-reranker-0.6b
cargo build --locked --release --features qwen3
LENS_EMBEDDING_BACKEND=qwen3 LENS_EMBEDDING_ARTIFACTS=models/qwen3-embedding-0.6b LENS_EMBEDDING_MANIFEST_SHA256=<digest> \
LENS_RERANKING_BACKEND=qwen3 LENS_RERANKING_ARTIFACTS=models/qwen3-reranker-0.6b LENS_RERANKING_MANIFEST_SHA256=<digest> \
  target/release/munarium-lens
```

Loading takes about a minute and needs about 6 GiB of memory for both
stages. Readiness then reports `"backend":"qwen3"`, `"retrieval_model":true`
and each manifest digest. Lens applies no instruction, so send the query
prefixes Server would apply, for example the JSON rerank `query`
`"<Instruct>: Given a web search query, retrieve relevant passages that answer the query\n<Query>: What grows in an orchard?"`.
The [backend design](../design/qwen3-backend.md) explains which parts of the
reranker prompt are Lens's template and which are Server's instruction.
A wrong digest, a changed file or an unsafe path leaves the stage `failed`
and logs one category such as `manifest_digest_mismatch`.

The opt-in test compares both models with their model cards:

```sh
LENS_QWEN3_TEST_DIR=models cargo test --release --features qwen3 -- --ignored --nocapture
```

## Fetching a published bundle

Once the maintainer has published a bundle, a build with the `fetch` feature
can download it from `huggingface.co/iokaio/munarium-lens-models`, falling
back to the `models/<bundle>` GitHub release. Neither location nor the order
can be changed. Take the commit and manifest digest from the
[model runtime decision](../design/model-runtime.md) or the release notes,
and use a cache directory outside the checkout; Lens refuses one inside a
Lens source tree.

```sh
cargo build --locked --release --features qwen3,fetch
target/release/munarium-lens fetch --bundle qwen3-embedding-0.6b-f32-1024d \
  --revision <40-char-commit> --manifest-sha256 <digest> --cache-dir ~/.cache/munarium-lens
```

The command prints the verified digest. Then either point
`LENS_EMBEDDING_ARTIFACTS` at `~/.cache/munarium-lens/qwen3-embedding-0.6b-f32-1024d`
or let Lens fetch at startup:

```sh
LENS_EMBEDDING_BACKEND=qwen3 LENS_EMBEDDING_BUNDLE=qwen3-embedding-0.6b-f32-1024d \
LENS_EMBEDDING_BUNDLE_REVISION=<40-char-commit> LENS_EMBEDDING_MANIFEST_SHA256=<digest> \
LENS_MODEL_CACHE_DIR=~/.cache/munarium-lens target/release/munarium-lens
```

A verified copy already in the cache loads without any connection. The
[README](../../README.md#downloading) describes the URLs, redirect hosts,
time limits and failure categories.

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

## Model container

`--build-arg LENS_FEATURES=qwen3,fetch` builds the model image: the Candle
backend and the fetcher, with no weights. Give it a writable volume at
`/var/cache/lens`, which the image creates owned by its non-root user, and
keep the root filesystem read-only:

```sh
docker build --build-arg LENS_FEATURES=qwen3,fetch -t munarium-lens-qwen3:local .
docker volume create lens-models
docker run --rm --read-only -p 127.0.0.1:8787:8787 -v lens-models:/var/cache/lens \
  -e LENS_HOST=0.0.0.0 -e LENS_ALLOW_UNAUTHENTICATED_NON_LOOPBACK=1 \
  -e LENS_MODEL_CACHE_DIR=/var/cache/lens \
  -e LENS_EMBEDDING_BACKEND=qwen3 -e LENS_EMBEDDING_BUNDLE=qwen3-embedding-0.6b-f32-1024d \
  -e LENS_EMBEDDING_BUNDLE_REVISION=<40-char-commit> -e LENS_EMBEDDING_MANIFEST_SHA256=<digest> \
  munarium-lens-qwen3:local
```

The same image runs `fetch` as a one-off job, for example in a Kubernetes
init container that fills a volume before the serving container starts:

```sh
docker run --rm --read-only -v lens-models:/var/cache/lens munarium-lens-qwen3:local \
  fetch --bundle qwen3-embedding-0.6b-f32-1024d --revision <40-char-commit> \
  --manifest-sha256 <digest> --cache-dir /var/cache/lens
```

On Kubernetes, mount an `emptyDir` (downloaded again on every new pod) or a
persistent volume claim (kept across restarts) at `/var/cache/lens`, set
`readOnlyRootFilesystem: true`, and allow egress on port 443 only to the hosts
listed in the [README](../../README.md#downloading), directly or through an
HTTP `CONNECT` egress proxy set with `HTTPS_PROXY` and `NO_PROXY` (see the
README for the accepted forms). A first start downloads about 1.2 GB per stage before readiness, so
use a startup probe with enough time. A failed fetch is retried only by
restarting the container (the approved restart-only policy). The image trusts the Mozilla root
certificates compiled into the binary (`webpki-roots`), not the base
image's certificate store. The same image also serves bundles mounted
read-only through `LENS_*_ARTIFACTS`; without bundle settings it makes no
connection. These are the only two images: the build refuses
`LENS_FEATURES=qwen3` alone and any other value.

Run the contributor checks in [Contributing](../../CONTRIBUTING.md) before
opening a pull request. See the [serving guide](../ops/serving.md) for
the proposed production lifecycle.
