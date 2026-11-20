# Lens decision register

| Decision | Status | Owner and closure |
|---|---|---|
| CPU scaffold | Implemented with Rust 1.98.1, Axum and Tokio, loopback only | Lens maintainers retain the lockfile and verify the no-weights CPU suite. Tokio is pinned at 1.53.3 for RUSTSEC-2026-0386. |
| LENS-1 serving and reference backend | Implemented; reference backend ready, model backend pending | Readiness, discovery, both operations, limits, overload refusal and a deterministic non-retrieval reference backend, with a reference-only image. The [implemented profile](endpoint-profile.md#lens-1-implemented-profile-clarifications-proposed) awaits Server IR-1 alignment and the hub export. |
| Production HTTP profile | Proposed; export pending | Server retrieval, Lens and hub owners publish IR-0/IR-1 types, error precedence and golden fixtures and prove consumer parity. |
| Model runtime | **Accepted 20 November 2026:** Candle, first on CPU behind the opt-in `qwen3` feature ([comparison](model-runtime.md)). The maintainer noted "we can alter that later if needed". | The Lens runtime owner records parity with the publisher's reference values and indicative CPU latency. LENS-2 measures it on approved hardware; a missed latency gate reopens the decision. |
| Runtime layout and build | Open | Lens runtime owner compares co-located and independent processes, pins the release runtime and base image and proves startup, isolation and load on the candidate. |
| Model pair and dimension | **Accepted 20 November 2026:** Qwen3-Embedding-0.6B and Qwen3-Reranker-0.6B at 1,024 dimensions ([Server record](https://github.com/iokaio/munarium/blob/89e1bc2019c80d697d2d8adc9237a9ce577cf3d7/server/docs/design/implementation-ledger.md#maintainer-decisions-20-november-2026), [pinned revisions](model-runtime.md#model-pair-candidates)) | Lens evaluation owner qualifies the pair against the required reference and hash baseline. Record upstream rights and the chosen artifacts at freeze. |
| Precision | Open | FP32 on CPU first. Reduced precision follows only after drift and quality pass in LENS-2. |
| Instruction prefixes | **Accepted 20 November 2026:** Server owns query, passage and rerank instructions; Lens applies none ([Server record](https://github.com/iokaio/munarium/blob/89e1bc2019c80d697d2d8adc9237a9ce577cf3d7/server/docs/design/implementation-ledger.md#maintainer-decisions-20-november-2026), [endpoint profile](endpoint-profile.md#lens-1-implemented-profile-clarifications-proposed)) | Each request string is the effective input. A model's fixed template, such as the reranker's chat frame, stays with Lens as part of the served identity. |
| Authenticated service transport and placement | Pending IR-1 and SC2 | Server and Lens operators choose and test credential provisioning, rotation, network isolation and dispatch policies. Hub owners record/export shared semantics. |
| Serving caps and supported hardware | Open | Lens operations owner records availability, cost authority, limits and expiry, then measures concurrency, tokens, cold start, memory and latency before publishing a profile. |
| Quality thresholds and instruction variants | Open | Lens evaluation and Server retrieval owners freeze per-family thresholds, datasets and the Server-applied instruction variants before scoring, then record I3 and LENS-D1-T3/T4. |
| Workspace qualification | Pending approved source pins and environments | Release owner assigns operators with corpus authority, procures permitted environments and collects all LENS-D2 receipts. |

The [Server decision register](https://github.com/iokaio/munarium/blob/0791d06f3020a8a0d06d646ca407c210b84721b8/server/docs/design/README.md#decision-register) owns
the shared release and compatibility requirements. The platform hub records
contract decisions. This register records progress and evidence in Lens.
