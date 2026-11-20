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

The optional `qwen3` cargo feature, off by default, adds Candle
(`candle-core`, `candle-nn`, `candle-transformers`; MIT OR Apache-2.0),
Hugging Face `tokenizers` (Apache-2.0), `sha2` (MIT OR Apache-2.0) and
`serde_json` as a normal dependency, with their transitive crates. Every
crate in that graph offers a permissive license; see the
[backend design](docs/design/qwen3-backend.md). A binary or image built with
the feature needs notices generated from that larger graph. The feature
loads Qwen3-Embedding-0.6B and Qwen3-Reranker-0.6B weights and tokenizers
(Apache-2.0 per their model cards) from an operator-supplied directory; the
repository and images contain none of them.

The optional `fetch` cargo feature, off by default, adds `ureq` and
`ureq-proto` (MIT OR Apache-2.0), `rustls` (Apache-2.0 OR ISC OR MIT),
`rustls-webpki` (ISC), `ring` (Apache-2.0 AND ISC; its source carries
BoringSSL-derived code under the terms in its license files),
`webpki-roots` (CDLA-Permissive-2.0: Mozilla's CA root certificates compiled
in as data), `untrusted` (ISC), `subtle` (BSD-3-Clause) and further MIT OR
Apache-2.0 crates, with `serde_json` and `sha2` as normal dependencies. A
binary or image built with `fetch` needs notices generated from that graph,
including the `ring` and `webpki-roots` license texts. The fetcher downloads
model bundles from the Munarium distribution locations at run time; the
bundles keep their upstream licenses and are not part of the binary or
image.

The [Dockerfile](Dockerfile) builds on the digest-pinned `rust:1.98.1-alpine`
image and copies the static binary into the digest-pinned
`gcr.io/distroless/static-debian12:nonroot` image. That runtime base carries
its own Debian package notices. The default reference image adds no model
runtime or HTTP client; `--build-arg LENS_FEATURES=qwen3,fetch`, the only
other image, adds the Candle and fetcher graphs above.

Before distributing a binary or image, generate and review notices from its
exact shipping dependency graph and runtime image, retain applicable license
texts, and record separate model/tokenizer and dataset rights before any
import or redistribution. See the [artifact design](docs/design/artifacts-and-serving.md).
