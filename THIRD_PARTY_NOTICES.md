# Third-party notices

The scaffold source includes a LICENSE and Apache-2.0 governance material
adapted from [Munarium](https://github.com/iokaio/munarium). The
[Contributor Covenant attribution](CODE_OF_CONDUCT.md#attribution) records
the conduct policy's upstream source.

Cargo resolves the dependencies listed in [Cargo.lock](Cargo.lock). The
direct dependencies are Axum (MIT), Serde (MIT OR Apache-2.0) and Tokio (MIT).
Upstream crate notices apply to downloaded sources. The repository ships
synthetic fixtures and no dependency source archive or model assets.

Before distributing a binary or image, generate and review notices from its
exact shipping dependency graph and runtime image, retain applicable license
texts, and record separate model/tokenizer and dataset rights before any
import or redistribution. See the [artifact design](docs/design/artifacts-and-serving.md).
