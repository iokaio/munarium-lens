# Lens decision register

| Decision | Status | Owner and closure |
|---|---|---|
| CPU scaffold | Implemented with Rust 1.98.1, Axum and Tokio, loopback only | Lens maintainers retain the lockfile and verify the no-weights CPU suite. Tokio is pinned at 1.53.3 for RUSTSEC-2026-0386. |
| LENS-1 serving and reference backend | Implemented; reference backend ready, model runtime pending decision | Readiness, discovery, both operations, limits, overload refusal and a deterministic non-retrieval reference backend, with a reference-only image. The [implemented profile](endpoint-profile.md#lens-1-implemented-profile-clarifications-proposed) awaits Server IR-1 alignment and the hub export. |
| Production HTTP profile | Proposed; export pending | Server retrieval, Lens and hub owners publish IR-0/IR-1 types, error precedence and golden fixtures and prove consumer parity. |
| Runtime layout and build | Open; [candidates proposed](model-runtime.md) | Lens runtime owner compares co-located and independent processes, pins release runtime/base image and proves startup, isolation and load on the candidate. |
| Models, dimension and precision | Open; [candidates proposed](model-runtime.md) | Lens evaluation owner compares Qwen3-Embedding-0.6B and Qwen3-Reranker-0.6B candidates against the required reference and hash baseline. Record upstream rights, chosen artifacts, precision and dimensions. Server's 1,024-dimension trial is proposed. |
| Authenticated service transport and placement | Pending IR-1 and SC2 | Server and Lens operators choose and test credential provisioning, rotation, network isolation and dispatch policies. Hub owners record/export shared semantics. |
| Serving caps and supported hardware | Open | Lens operations owner records availability, cost authority, limits and expiry, then measures concurrency, tokens, cold start, memory and latency before publishing a profile. |
| Quality thresholds and instructions | Open | Lens evaluation and Server retrieval owners freeze per-family thresholds, datasets and instruction variants before scoring, then record I3 and LENS-D1-T3/T4. |
| Workspace qualification | Pending approved source pins and environments | Release owner assigns operators with corpus authority, procures permitted environments and collects all LENS-D2 receipts. |

The [Server decision register](https://github.com/iokaio/munarium/blob/0791d06f3020a8a0d06d646ca407c210b84721b8/server/docs/design/README.md#decision-register) owns
the shared release and compatibility requirements. The platform hub records
contract decisions. This register records progress and evidence in Lens.
