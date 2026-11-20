# Changelog

## Unreleased

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

- Add the LENS-0 loopback CPU scaffold with liveness, typed unavailable responses,
  and an offline HTTP lifecycle test.
- Document the proposed profiles, artifact verification, operations and Server v1.5
  qualification. Add contributor governance and required CI entry points.
