# Lens design and implementation plan

Lens LENS-0 provides a buildable CPU scaffold. Begin production implementation
when the relevant ClearView Server v1.5 interfaces are available and shared
semantics have been recorded and exported through the platform hub. Test the
completed Lens image against the Server v1.5 release candidate before Server
ships. Enrollment remains explicit. An upgraded installation can keep its v1.4
hash indexes and existing provider settings. [ClearView plan](https://github.com/iokaio/munarium/blob/0791d06f3020a8a0d06d646ca407c210b84721b8/server/docs/design/README.md) ·
[Lens handoff](https://github.com/iokaio/munarium/blob/0791d06f3020a8a0d06d646ca407c210b84721b8/server/docs/design/repository-handoffs.md#lens-foundation-lens-0)

## Responsibilities

| Owner | Boundary |
|---|---|
| Server | Govern collection enrollment, index identity and approval, retention and placement, authorization, caller attribution, attempt accounting, fusion, result and citation validation, and release of evidence. The provider gateway owns endpoint transport, deadline, credentials and metering. |
| Lens | Serve the requested embedding or reranking profile under a pinned identity. Verify assets, authenticate service callers, enforce input and resource limits, report readiness and usage known to the backend, and return bounded results or typed failures. |
| Operator | Authorize processing and placement, supply verified hardware and assets, choose per-collection policies in Server, provision approved pools, and maintain the previous approved profiles until rollback is verified. |
| Platform contract owners | Record and export shared semantic decisions before consumer implementation. Re-vendor the versioned contract and compare fixtures across the exported contract, Server, Lens and an independent endpoint. |

Embedding changes require a new compatible index. Reranking applies only to the
authorized candidates already selected by Server. It reorders that set and can
change independently of the embedding index. Citations and source identity stay
under Server authorization. Text generation, chat, ingestion, and MCP routing
are beyond Lens's scope. MCP calls that use enrolled search follow Server's
durable admission and accounting before any Lens request. [Server inference
boundary](https://github.com/iokaio/munarium/blob/0791d06f3020a8a0d06d646ca407c210b84721b8/server/docs/design/airk-analysis.md#86-the-seam-an-inference-endpoint-behind-the-provider-gateway) ·
[MCP admission](https://github.com/iokaio/munarium/blob/0791d06f3020a8a0d06d646ca407c210b84721b8/server/docs/design/mcp-analysis.md#a6)

## Milestones and implementation order

| Milestone | Work and exit condition | Status |
|---|---|---|
| LENS-0 | Pin the Rust toolchain and build graph. Check the loopback stub, content-free errors, lifecycle, indexed design and open-source contributor workflow. Collect LENS-D1-T1. | Scaffold |
| LENS-1 | Use the IR-0 types and IR-1 probe to implement the published profiles. Pin and verify imported artifacts, implement one backend adapter per stage, apply service authentication and resource limits, build an offline-serving image, and collect LENS-D1-T2 with the actual Server probes. | Reference backend ready; model pair and Candle runtime accepted; model backend in progress. Serving, limits, overload refusal and the reference-only image are in place. Artifact import, service authentication, the model adapter and LENS-D1-T2 remain. |
| LENS-2 | With IR-2–IR-4 available, measure the candidate precision, hardware, throughput and quality. Exercise I1, I2, SC1 and SC2. Meet both I3 thresholds and collect LENS-D1-T3. | Pending working image, approved environments and Server integration |
| LENS-3 | Compare retrieval instructions to publisher defaults on held-out corpora, pin the winning settings under their own identity and rerun affected qualification. Collect LENS-D1-T4. | Pending frozen judgments and LENS-2 evidence |
| Workspace release | Build and exercise every root with the chosen profile, resolve citations and verify rollback. Record LENS-D2-T1–T4 and a v1.4 upgrade with Lens disabled on the exact release pair. | Pending qualified profile and authorized corpus freeze |

The Server gate for production serving is IR-0–IR-4. IR-0 follows the R1
retrieval disclosure semantics. IR-1 adds the provider kind, per-capability
models and positive probe. IR-2 builds semantic indexes side by side and uses
fenced, cancellable builds with fair queueing and progress for large
rebuilds (R4, R6, R8, R9). IR-3 wires reranking after fusion. IR-4 requires R7
admission and verified indexes and policies before enrollment. While these land,
prepare synthetic fixtures and identify environments and licenses. Freeze the
actual candidate revisions before live qualification. [IR sequence](https://github.com/iokaio/munarium/blob/0791d06f3020a8a0d06d646ca407c210b84721b8/server/docs/design/airk-analysis.md#812-implementation-sequence)

## Implementation work packages

1. **Close the wire profile.** For embed and rerank independently, agree on
   method, normalized base URL and route, JSON success/failure variants, model
   discovery, allowed inputs, identity signaling, limits, usage, error
   precedence and authentication at IR-0/IR-1. Publish vectors for reversed
   embedding rows, duplicate and missing indices, wrong dimensions, nonfinite
   numbers, tied scores and whole-batch failure. Run the Server consumer
   against both Lens and an independently implemented endpoint. Replace the
   temporary HTTP request types when the export is available.
   [Endpoint profile](design/endpoint-profile.md)
2. **Implement artifact import and serving identity.** Define a versioned
   manifest and canonical digest method. Add an explicit importer that verifies
   files and installs an immutable directory atomically. At offline startup,
   verify expected digests, open only files from the verified directory, load
   the configured stage, run its known input probe and publish readiness only
   when the profile matches. Fail startup or leave the stage unready on a bad
   manifest, missing file, unsupported device or incorrect probe. Keep the
   failure reason content-free. [Artifact design](design/artifacts-and-serving.md)
3. **Implement runtime adapters.** Compare one service hosting both stages
   against one process per stage. Start with one adapter for each stage and
   expose both through independently routed pools. Implement a fallible async
   boundary around the blocking model work. Tokenize the effective inputs
   with the pinned tokenizer and templates, validate lengths before inference,
   bound batch tokens and queueing, and preserve input indices through any
   batch split. Validate finite vectors and scores before serializing a
   complete result. Do not release partial outputs after failed work. Pin the
   measured runtime, base image and device dependencies in the shipping build.
4. **Add authenticated admission and telemetry.** Authenticate the Server
   service identity before accepting inference work. Validate stage and
   identity, body/count/token caps and the remaining deadline before
   dispatch. Bound the queue, active work and shutdown drain separately per
   stage. Report overload before dispatch when capacity cannot be obtained.
   Emit bounded labels for stage, profile and outcome and record queue,
   execution, refused and completed work. Keep results and inputs out of logs,
   traces and metrics. Disable input/output caching initially. [Replica design](design/replication.md)
5. **Integrate in Server.** Probe the selected endpoint and record the approved
   versioned binding, including separate stage identities and a fingerprint.
   Build with the embedding identity pinned to the index. Validate the whole
   embedding response before caching or writing any vectors. In a query,
   resolve authorized pinned indexes first and group only compatible inputs
   with the same authorized model, instructions, placement and payer.
   Rerank after authorization and fusion. Within each group reorder only that
   group's occupied positions and preserve the original passages and citations.
   Use fresh sessions for enrollment and rollback checks.
6. **Qualify the image and operational profile.** On a disposable, authorized
   deployment, exercise missing and corrupt artifacts, an invalid embedding
   batch, rerank omission, overload, timeout, drift, worker removal, Server
   retry and caller disconnect, and authorized and denied placements. Then
   evaluate precision, quality and latency on predeclared data and thresholds.
   Freeze the candidate for file-by-file workspace receipts. Publish the
   qualified image identity, installation and rollback guidance, and only
   evidence approved for the public release record. [Qualification](design/qualification.md)

## Enrollment, accounting and rollback

For each configured stage, Server defaults enrolled reads to I1 `require`.
On stage failure, return a typed refusal. Explicit `prefer` permits a
disclosed lexical-only read for embedding loss or unchanged fused order for
rerank loss. A failed build cannot activate an incomplete index. An
authorized shadow outcome goes only to the comparison record. Partial
multi-collection reads still follow Server's C1 evidence rules.

Under I2, Server reserves against the collection owner's tenant budget before
each physical attempt. Track the initiating caller through queued builds, use
one request deadline across queue and attempts, charge a shared call once,
meter authorized shadow work separately and reconcile once per attempt.
Preserve unknown usage after an indeterminate response. Lens exposes known
usage when the contract permits it. Pools and retries create no new spending
authority. An unaudited MCP execution or denied budget admission results
in zero inference requests. [I1 and I2](https://github.com/iokaio/munarium/blob/0791d06f3020a8a0d06d646ca407c210b84721b8/server/docs/design/airk-analysis.md#i1)

Promote a new model under a distinct served name and keep the previous
approved index, binding and pool available. For rollback, restore the previous
index and policy using Server's revisioned management operation, verify the
active pins, open fresh sessions and replay baseline queries with Lens
unavailable. Resolve previous citations under current authorization. Verify
mixed-version and binary rollback against the Server release guide before
activating any state that v1.4 cannot use. [Scaling and rollback](ops/scaling.md)

## Implementation decisions still open

The production runtime, model artifacts and licenses, hardware profile, maximum
bytes/tokens, queue/concurrency/deadlines, instruction variants, precision and
per-family quality/latency thresholds require measured evidence. The LENS-1
default limits only govern local checks. The [proposed runtime and model
comparison](design/model-runtime.md) lists the candidates and what qualification
measures. Track owners and exit
criteria in the [decision register](design/decisions.md). Keep required but
unavailable coverage visible in the release record.
