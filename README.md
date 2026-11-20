# Munarium Lens

Munarium Lens is the planned self-hosted embedding and passage reranking endpoint
for [Munarium Server](https://github.com/iokaio/munarium). It will let operators
place retrieval inference on their own hardware and size it independently of Server.

**Status — LENS-0 scaffold verified. LENS-1: reference backend ready; model
runtime pending decision.** The Rust service serves readiness, model discovery,
embeddings and reranking with request limits and typed refusals. Its only
backend is a deterministic reference implementation for tests and protocol
checks. It is **not a retrieval model**. The model runtime and first model pair
are an open [proposed decision](docs/design/model-runtime.md), so LENS-1 is not
complete. Both stages are disabled and the listener is loopback-only by default.
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
