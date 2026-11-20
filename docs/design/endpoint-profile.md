# Proposed endpoint profile

The [Server inference seam](https://github.com/iokaio/munarium/blob/0791d06f3020a8a0d06d646ca407c210b84721b8/server/docs/design/airk-analysis.md#86-the-seam-an-inference-endpoint-behind-the-provider-gateway)
proposes separate OpenAI-style embedding and reranking profiles. Confirm routes,
discovery and response envelopes with the hub export and pinned IR-1 probe. The
scaffold accepts a minimal synthetic subset to exercise typed unavailability.

| Proposed operation | Request | Successful result |
|---|---|---|
| `POST /v1/embeddings` | Exact served `model`, ordered batch of text in `input`, and only the input forms defined by IR-0. Bind passage and query preprocessing to the approved profile. | Embedding rows in `data` with zero-based `index` and a finite `embedding` at the exact declared dimension. Return model and usage under the approved profile. |
| `POST /v1/rerank` | Exact served `model`, `query`, and ordered `documents`. Preserve the full authorized candidate set. | `results` contains every zero-based input `index` once, with finite `relevance_score`. Sort descending by score, breaking ties by original position. Usage and model identity follow the approved profile. |

IR-1's generic probe verifies discovery for both models, the declared
dimension and finite embedding values, and complete rerank permutations.
Resolve the configured HTTP(S) base URL once. Support the approved `/v1`
prefix form without appending it twice, reject embedded credentials, query
and fragment, and test a prefixed private endpoint. Names must distinguish
artifact and profile revisions. An enrollment probe must expire on endpoint
or binding drift and require requalification. `/healthz` observes process
liveness and `/readyz` observes verified loaded profiles. Approve stage
discovery and the per-pool readiness representation together.

## Validation and refusal

1. Authenticate the service principal. Reject unauthorized callers before
   reading a body or reporting model availability.
2. Enforce the agreed media type, byte and item caps, schema and identity
   routing. Tokenize exact effective inputs including prefixes, preprocessing,
   special tokens and rerank pair templates. Reject unknown profiles or bounds
   violations without inference. Overlong requests are refused as a whole.
3. Check readiness, remaining deadline and available capacity. Bound queue
   wait within the same deadline as execution. Agree on refusal precedence,
   status, stable problem codes, and usage for work started before a deadline.
   The export must include a case with more than one possible failure.
4. Validate counts, indices, dimensions and finite values on Lens output.
   Restore embedding order from validated indices and require a bijection
   before writing or caching vectors. Require a full rerank permutation;
   Server uses the order of original candidates and ignores returned passage
   content. A malformed reply fails the stage under I1.

Versioned fixtures should cover wrong media, malformed JSON, wrong stage or
identity, byte/item/token caps and boundary values, duplicate and out-of-range
indices, missing indices, reversed indexed embedding rows, dimension mismatch,
overflow/nonfinite values, stable rerank ties and an unavailable stage. Confirm
that the Server consumer preserves citation identity after reordering. The
[local stub probes](../guides/local-development.md) currently cover the refusal
path without model discovery or successful inference.
