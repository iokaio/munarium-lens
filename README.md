# Munarium Lens

Munarium Lens is the planned self-hosted embedding and passage reranking endpoint
for [Munarium Server](https://github.com/iokaio/munarium). It will let operators
place retrieval inference on their own hardware and size it independently of Server.

**Status — LENS-0 scaffold verified. LENS-1: reference backend ready; model
backend in progress.** The Rust service serves readiness, model discovery,
embeddings and reranking with request limits and typed refusals. Its only
backend is a deterministic reference implementation for tests and protocol
checks. It is **not a retrieval model**. On 20 November 2026 the maintainer
accepted Qwen3-Embedding-0.6B with Qwen3-Reranker-0.6B at 1,024 dimensions,
with instruction prefixes owned by Server, and confirmed
[Candle](docs/design/model-runtime.md) as the runtime. The model backend is not
yet qualified, so LENS-1 is not complete. Both stages are disabled and the
listener is loopback-only by default.
The tests need no model weights, GPU, credentials or Server.

```sh
cargo test --locked --offline
LENS_EMBEDDING_BACKEND=reference LENS_RERANKING_BACKEND=reference cargo run --locked --offline
```

See the [local development walkthrough](docs/guides/local-development.md) for
the probe commands, configuration and the reference container. The [design and implementation plan](docs/design-plan.md)
tracks the Server v1.5 dependencies, implementation milestones and release receipts.
The [documentation index](docs/README.md) links the endpoint, artifact, serving and
qualification designs.

[Changelog](CHANGELOG.md) · [Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) ·
[Support](SUPPORT.md) · [Apache-2.0 license](LICENSE)
