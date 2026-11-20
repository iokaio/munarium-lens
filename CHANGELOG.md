# Changelog

## Unreleased

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
