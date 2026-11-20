# Changelog

## Unreleased

- Add the optional `fetch` cargo feature, off by default (maintainer decision
  of 20 November 2026). It downloads a model bundle pinned by name, full
  Hugging Face commit and manifest SHA-256, only from the fixed
  `huggingface.co/iokaio/munarium-lens-models` repository and, if that fails,
  the fixed `models/<bundle>` GitHub release of this repository. The base URLs
  and the order are compile-time constants with no setting or option to change
  them; redirects are followed only over HTTPS to each location's documented
  download hosts, and proxy variables are ignored. The manifest is fetched and
  checked first with the shared manifest code; only listed files follow, each
  streamed to a temporary file capped at its listed size, checked, flushed and
  renamed into place. Verified files are reused, so a complete cache makes no
  connection. Use it through the new `munarium-lens fetch` subcommand, which
  prints the verified manifest digest, or at startup with
  `LENS_*_BUNDLE`, `LENS_*_BUNDLE_REVISION`, `LENS_MODEL_CACHE_DIR` and the
  optional `LENS_MODEL_FETCH_TIMEOUT_SECS`. A stage whose bundle cannot be
  fetched and verified stays `failed`. Without these settings, behavior is
  unchanged and Lens makes no outbound connection. Dependencies: `ureq` 3.5.2
  on `rustls` 0.23.46 with the `ring` 0.17.14 provider and `webpki-roots`
  1.0.12; `deny.toml` admits exactly these through their expected parents and
  adds a scoped CDLA-Permissive-2.0 license exception for `webpki-roots`. The
  Dockerfile accepts `LENS_FEATURES=qwen3,fetch` for the model image, creates
  a non-root-owned `/var/cache/lens` for a cache volume, and the reference
  image still has no HTTP client or TLS stack. The `fetch` subcommand is
  accepted only as the first argument; any build without the feature refuses
  it. Manifest verification now streams files that a stage does not load
  instead of holding them in memory.
- Document model and weight distribution in the README. Bundles are hosted at
  `huggingface.co/iokaio/munarium-lens-models`, with a GitHub Releases backup
  under `models/<served_model>` tags. Each bundle has a fixed layout and a
  permanent served name, and is pinned by Hugging Face commit and manifest
  SHA-256. Both locations go through the same manifest verification. The
  document covers the planned opt-in fetcher (off by default, explicit, atomic
  writes, no credentials, feature-gated so the serving build keeps no HTTP or
  TLS stack), manual fetching until then, and the maintainer publishing
  procedure. The hosting location was decided on 20 November 2026; download
  support followed in the `fetch` entry above.
- Add `deny.toml` and an advisory `supply chain` workflow (maintainer-approved
  on 20 November 2026) that runs a pinned, checksum-verified cargo-deny 0.20.2
  against the default and all-features graphs on pushes to `main`, pull
  requests and weekly. The policy allows the Server's permissive license
  list, denies every RustSec advisory, yanked crates, wildcard requirements,
  TLS stacks, HTTP clients and the C tokenizer backends, and allows only
  crates.io. One exception, RUSTSEC-2024-0436 for the compile-time `paste`
  macro in the `qwen3` graph, is recorded with its reasoning and awaits
  maintainer acceptance.
- Add the optional `qwen3` cargo feature, off by default: Qwen3-Embedding-0.6B
  and Qwen3-Reranker-0.6B on Candle 0.11.1, CPU, FP32. Embedding uses
  last-token pooling and L2 normalization at 1,024 dimensions; reranking uses
  the model card's chat template and returns `logit(yes) - logit(no)`. Lens
  adds no instruction text. Each stage loads only from an operator-supplied
  directory whose `lens-manifest.json` matches a configured SHA-256 and whose
  listed files match their sizes and digests. Readiness then reports
  `retrieval_model: true` and `manifest_sha256`. New variables:
  `LENS_*_ARTIFACTS` and `LENS_*_MANIFEST_SHA256`; `scripts/lens_manifest.py`
  writes a manifest. Model loading now runs off the async runtime. The
  default build and its dependency graph are unchanged. The Dockerfile accepts
  `--build-arg LENS_FEATURES=qwen3`, and its image labels now name the
  features. Indicative parity and latency from a development laptop are in
  the [backend design](docs/design/qwen3-backend.md).
- Record the maintainer decisions of 20 November 2026: Qwen3-Embedding-0.6B
  and Qwen3-Reranker-0.6B at 1,024 dimensions are accepted, and Server owns
  instruction prefixes while Lens applies none. The maintainer also confirmed
  Candle as the runtime, which can be altered later if needed. Update the
  model runtime page, decision register, endpoint profile and milestone status
  (LENS-1: reference backend ready; model backend in progress).
- Update the pinned Tokio release from 1.53.1 to 1.53.3, which resolves
  [RUSTSEC-2026-0386](https://rustsec.org/advisories/RUSTSEC-2026-0386).
  No other locked package changes.
- LENS-1 serving: add readiness with served identity, dimensions and limits,
  OpenAI-style model discovery at `GET /v1/models`, embeddings and full-permutation
  reranking behind per-stage backend interfaces. Add per-stage item, token and
  batch bounds, bounded admission with a typed `overloaded` refusal, output
  validation, and `LENS_*` configuration. Both stages stay disabled and the
  listener stays on loopback by default.
- Add the deterministic reference backend (`LENS_*_BACKEND=reference`) for tests
  and protocol qualification. It is not a retrieval model and reports
  `"retrieval_model": false`. The model runtime is pending a maintainer decision.
- Add a Dockerfile for the reference-only image: a static musl build on
  digest-pinned `rust:1.98.1-alpine` and `distroless/static-debian12:nonroot`
  bases, with no model runtime or weights.
- Document the implemented LENS-1 profile in the endpoint design as proposed
  clarifications for Server IR-0/IR-1: discovery and readiness shapes, accepted
  request fields, usage, refusal codes and their order. Update the walkthrough,
  serving guide, decision register and milestone status (LENS-1: reference
  backend ready; model runtime pending decision).
- Add the LENS-0 loopback CPU scaffold with liveness, typed unavailable responses,
  and an offline HTTP lifecycle test.
- Document the proposed profiles, artifact verification, operations and Server v1.5
  qualification. Add contributor governance and required CI entry points.
