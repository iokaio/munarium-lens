# Munarium Lens

Munarium Lens is the planned self-hosted embedding and passage reranking endpoint
for [Munarium Server](https://github.com/iokaio/munarium). It will let operators
place retrieval inference on their own hardware and size it independently of Server.

**Status — scaffold.** The Rust service runs on loopback and reports liveness.
Readiness and both inference routes return typed HTTP 503 responses. The scaffold
uses synthetic inputs and needs no model weights, GPU, credentials or Server.

```sh
cargo test --locked
cargo run --locked
```

See the [local development walkthrough](docs/guides/local-development.md) for
the probe commands and expected responses. The [design and implementation plan](docs/design-plan.md)
tracks the Server v1.5 dependencies, implementation milestones and release receipts.
The [documentation index](docs/README.md) links the endpoint, artifact, serving and
qualification designs.

[Changelog](CHANGELOG.md) · [Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) ·
[Support](SUPPORT.md) · [Apache-2.0 license](LICENSE)
