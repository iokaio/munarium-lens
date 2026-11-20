# Qualification and release receipts

## CPU and serving qualification

| Receipt | Exercise | Required evidence |
|---|---|---|
| LENS-D1-T1 | Fresh checkout. Build and run the Rust tests, start the process on loopback, probe both refusal routes and stop cleanly. | Lockfile and toolchain, exact source revision, observed responses and offline test outcome. |
| LENS-D1-T2 | Start the image offline with verified assets on named hardware. Run the candidate Server's actual embedding and rerank probes and the independent adapter fixtures. | Image digest, Server and Lens revisions, model/tokenizer digests, dimensions, runtime, device and license records, installation and rollback procedure. |
| LENS-D1-T3 / I3-T1–T2 | Compare approved precision to the reference and retrieval baseline. Qualify documentation and code independently. Include overlong requests, corrupt/missing assets and endpoint timeout. | Separately passing family receipts for one exact profile, peak memory, cold and warm latency, overlap of query and build work, and observed errors and authorization. |
| LENS-D1-T4 | Compare independently pinned query, passage and reranking instructions with publisher defaults using held-out examples. | Measured gain or a recorded decision to retain defaults. A new weight revision receives a fresh profile and qualification. |
| SC1-T1–T3 / SC2-T1–T3 | Resize Server first with Lens disabled, vary each stage's pool with Lens enabled, then drain, replace and remove a stage under load. Exercise shared and dedicated placement. | Test counts, hardware, ready/desired replicas, membership proof, latency and drain, placement, I1 refusal/fallback, I2 attempts, and restored pins and citations. |

Before any named run, record environment owner and operator, dated
availability evidence, access, processing and corpus approvals, cost cap
including retries, reservation/access expiry and topology. Pin driver and
runtime, precision, model and tokenizer, index/build settings, stage limits,
cutoff, load mix, seed, judgments and pass thresholds. Measure recall@k,
nDCG@k, p95 latency, failures, citations, denied access, peak memory,
repeatability and throughput. Compare hash retrieval, embed only, rerank
only and both stages. Use identical frozen inputs per family and freeze
judgments before scoring. Separate held-out quality results from per-file
coverage. Missing or failing required evidence holds release. Server's
[quality rule](https://github.com/iokaio/munarium/blob/0791d06f3020a8a0d06d646ca407c210b84721b8/server/docs/design/airk-analysis.md#i3) and
[environment requirements](https://github.com/iokaio/munarium/blob/0791d06f3020a8a0d06d646ca407c210b84721b8/server/docs/design/README.md#qualification-environments)
define the release gate.

## Workspace exercise

Use the handoff's [16-root inventory](https://github.com/iokaio/munarium/blob/0791d06f3020a8a0d06d646ca407c210b84721b8/server/docs/design/repository-handoffs.md#workspace-inventory)
and produce separate results for each root across all four stages.

1. **LENS-D2-T1 — freeze.** Resolve the authorized ref to a commit, pin its
   tree, and enumerate exact Git tree entries with NUL-safe mode/type/blob/size/path
   records. Classify each regular blob as documentation, source or excluded
   with reason. Review unknown types, pointers, gitlinks and symbolic links,
   and stage only authorized Git objects. Reconcile all 16 distinct roots with
   the manifest and classify or explain every source.
2. **LENS-D2-T2 — build.** Run each owning repository's documented build and
   applicable docs targets. Record commands and outcomes. Verify a v1.4-compatible
   baseline over the frozen input, saving approved index IDs, policies and
   results for rollback. Build semantic indexes alongside them, reconcile the
   source identities, extraction counts, embedding/profile and verified artifact
   pins, assign a disposition for non-extractable files, then approve.
3. **LENS-D2-T3 — query and cite.** Enroll with fresh sessions and use
   content-derived queries covering every content-bearing document and code
   file. Require observed embedding and rerank receipts. Resolve the expected
   citation through Server's authorized reference route. Check passage,
   repository/revision/path and a location when extraction supplies it.
   Cover same-named files, cross-family questions and denied access. Store
   expected answers and judgments outside searchable input and model mounts.
4. **LENS-D2-T4 — roll back.** Restore prior approved index and policy for
   every collection. With Lens unavailable, open fresh sessions and replay the
   baseline, comparing index IDs, ordering and resolved citations. Verify
   historical references under current authorization and reconcile outstanding
   attempts. Record coverage per file and unresolved failures per root.

The run manifest records schema, run ID, exact candidate and baseline
revisions/digests, authority, environment, profiles and policies. Each root
records repository identity, commit/tree, access class, authorized collection,
counts and bytes by family, actual indexed/chunk/token counts, exclusions,
and stage outcomes. Each file records path, mode, blob, family/disposition,
extraction, source/index IDs, query and expected citation references,
resolved citation and rollback verification. Report expected, indexed,
queried, cited and restored totals per root.

For restricted material, the corpus owner authorizes selection, users,
processing location, retention, endpoint and evidence venue before reading
or inference. Use a separate authorized environment and collection and the
approved dedicated pool as required. Keep private commit IDs, inputs, paths,
citations, logs and detailed receipts within that boundary. Export only
owner-approved status and an opaque receipt reference. Mark unavailable
stages as unavailable and keep the gate open.

Each acceptance ID and environment gets a receipt with date, accountable
owner, candidate and baseline digests, fixture/manifest revision, configuration,
expected and observed outcomes, coverage gaps and evidence location. Record
an upgrade with Lens disabled and the prior hash indexes, ranking and
configuration intact. Link the same exact candidate receipts from the Lens and
Server release records. [Server release matrix](https://github.com/iokaio/munarium/blob/0791d06f3020a8a0d06d646ca407c210b84721b8/server/docs/design/README.md#release-evidence)
