# Proposed endpoint profile

The [Server inference seam](https://github.com/iokaio/munarium/blob/0791d06f3020a8a0d06d646ca407c210b84721b8/server/docs/design/airk-analysis.md#86-the-seam-an-inference-endpoint-behind-the-provider-gateway)
proposes separate OpenAI-style embedding and reranking profiles. Confirm routes,
discovery and response envelopes with the hub export and pinned IR-1 probe.
The [LENS-1 section](#lens-1-implemented-profile-clarifications-proposed)
records the routes, shapes, limits and refusal order the service implements.

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
[local walkthrough](../guides/local-development.md) probes discovery, both
operations and the refusals against the reference backend.

## LENS-1 implemented profile (clarifications, proposed)

> **For Server IR-0/IR-1 implementers.** This section records what the
> LENS-1 service actually does. It refines the proposal above and does not
> change its routes or success shapes. Items marked *clarification* fill gaps
> the proposal left open; Server's consumer should match them or raise a
> hub decision. Everything here remains proposed until the hub export.

**Routes.** Paths are absolute; a Server base URL ending in `/v1` must not
append `/v1` again.

| Route | Result |
|---|---|
| `GET /healthz` | 204, no body. Process liveness only. |
| `GET /readyz` | *Clarification.* 200 with a JSON body when every configured stage loaded and passed its startup probe and at least one stage is ready; otherwise 503 `{"code":"backend_not_ready"}`. |
| `GET /v1/models` | *Clarification.* 200 OpenAI-style list of the **ready** stages only; an empty `data` array when none is ready. |
| `POST /v1/embeddings` | Embedding rows, below. |
| `POST /v1/rerank` | Full permutation, below. |

Readiness body, one entry per ready stage (`dimensions` only for embedding):

```json
{"status":"ready","stages":[
  {"stage":"embedding","model":"lens-reference/embedding-v1-1024d","backend":"reference",
   "retrieval_model":false,"dimensions":1024,"manifest_sha256":null,
   "limits":{"max_body_bytes":262144,"max_items":64,"max_input_tokens":512,"max_batch_tokens":16384}},
  {"stage":"reranking","model":"lens-reference/rerank-v1","backend":"reference",
   "retrieval_model":false,"manifest_sha256":null,"limits":{"max_body_bytes":262144,"max_items":64,"max_input_tokens":512,"max_batch_tokens":16384}}]}
```

Discovery entries are `{"id","object":"model","owned_by":"munarium-lens","stage","backend","retrieval_model"}`
plus `dimensions` for embedding. `manifest_sha256` is the lowercase hex
canonical manifest digest once artifact verification exists, and `null` for a
backend that loads no artifacts. A probe that requires a retrieval model must
reject `"retrieval_model": false`.

**Requests.** *Clarification.* Bodies are `application/json` objects with no
unknown fields. Embedding takes `model` and `input`, a non-empty **array** of
strings; a bare string, `encoding_format`, `dimensions` and `user` are
refused. Rerank takes `model`, a non-empty `query` and a non-empty `documents`
array of strings; `top_n` and `return_documents` are refused because the
whole candidate set is always scored. Empty strings are allowed as items.
Lens applies no instruction prefix or truncation: each string is the
effective input. Whether a model profile's query instruction is applied by
Server or by Lens remains open.

**Success.** Embedding: `{"object":"list","model":<served>,"data":[{"object":"embedding","index":i,"embedding":[...]}...],"usage":{"prompt_tokens":n,"total_tokens":n}}`
with rows in input order and every vector at the declared dimension.
Rerank: `{"model":<served>,"results":[{"index":i,"relevance_score":s}...],"usage":{"prompt_tokens":n,"total_tokens":n}}`,
every input index exactly once, sorted by descending score with ties in
original order (`-0.0` ties `0.0`). Results carry no `document` text.
`usage` counts the backend's tokens for the exact effective inputs; a rerank
pair counts query plus document, including any template.

**Refusals.** Every refusal body is `{"code":...,"stage":...}` with no request
content; readiness omits `stage`. Checks run in this order and the first
failure wins:

| Order | Status and `code` | Condition |
|---|---|---|
| 1 | *(reserved)* | Service authentication, once IR-1 chooses a scheme. Not implemented. |
| 2 | 415 `unsupported_media_type` | Missing or non-JSON content type. |
| 3 | 413 `body_too_large` | Body above `max_body_bytes`. |
| 4 | 400 `invalid_request` | Malformed JSON, unknown or missing field, wrong type, empty `model`, `input`, `query` or `documents`. |
| 5 | 503 `inference_unavailable` | Stage disabled, failed its startup probe, or the backend reported unavailability during work. |
| 6 | 404 `model_not_found` | `model` is not the stage's exact served name. |
| 7 | 413 `too_many_items` | More items than `max_items`. Checked before tokenization. |
| 8 | 413 `input_too_long` | One input, or one query and document pair, above `max_input_tokens`. The whole request is refused; nothing is truncated. |
| 9 | 413 `batch_too_large` | Total tokens above `max_batch_tokens`. |
| 10 | 503 `overloaded` with `Retry-After: 1` | No execution slot and the bounded queue is full, or the queue wait exceeded its timeout. Refused before dispatch. |
| 11 | 500 `inference_failed` | The backend panicked or returned a wrong count, wrong dimension or a non-finite value. No partial result is sent. |

Example with more than one failure: a request naming another model with too
many items returns 404 `model_not_found`; a request with too many items and an
overlong item returns 413 `too_many_items`. Usage for refused work is not
reported. Lens does not yet read a caller deadline; queue wait is bounded by
`LENS_QUEUE_TIMEOUT_MS`. Agreeing on a deadline header is open.

**Limits and admission.** Each stage has its own execution slots
(`LENS_MAX_IN_FLIGHT`) and queue (`LENS_MAX_QUEUE`). The defaults above are
placeholders for local checks, not measured capacity. Limits apply per
process; a pool's capacity is the sum of its ready workers.

**Reference backend.** `LENS_EMBEDDING_BACKEND=reference` and
`LENS_RERANKING_BACKEND=reference` serve a deterministic, fully specified
algorithm (see `src/reference.rs`) so an independent implementation can
reproduce exact vectors and scores for golden fixtures. It is not a retrieval
model. Its served names include the dimension, for example
`lens-reference/embedding-v1-1024d` (`LENS_REFERENCE_DIMENSIONS`).
