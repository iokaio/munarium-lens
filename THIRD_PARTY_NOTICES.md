# Third-party notices

The scaffold source includes a LICENSE and Apache-2.0 governance material
adapted from [Munarium](https://github.com/iokaio/munarium). The
[Contributor Covenant attribution](CODE_OF_CONDUCT.md#attribution) records
the conduct policy's upstream source.

Cargo resolves the dependencies listed in [Cargo.lock](Cargo.lock). The
direct dependencies are Axum (MIT), Serde (MIT OR Apache-2.0) and Tokio (MIT).
The tests also use serde_json (MIT OR Apache-2.0), which Axum already
brings into the graph. Upstream crate notices apply to downloaded sources.
The repository ships synthetic fixtures and no dependency source archive or
model assets. The deterministic reference backend is original code and uses
no model, tokenizer or dataset.

The [Dockerfile](Dockerfile) builds on the digest-pinned `rust:1.98.1-alpine`
image and copies the static binary into the digest-pinned
`gcr.io/distroless/static-debian12:nonroot` image. That runtime base carries
its own Debian package notices. The reference image adds no model runtime.

Before distributing a binary or image, generate and review notices from its
exact shipping dependency graph and runtime image, retain applicable license
texts, and record separate model/tokenizer and dataset rights before any
import or redistribution. See the [artifact design](docs/design/artifacts-and-serving.md).
